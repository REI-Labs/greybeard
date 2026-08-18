use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::Router;
use serde_json::{json, Value};

use crate::config::Config;
use crate::github::{Github, PrRef};
use crate::llm::Llm;
use crate::pipeline::review::{self, ReviewArgs};
use crate::telemetry::Telemetry;

/// How long a PR gets to settle after an event before its review starts —
/// rapid pushes collapse into one run (the newest event wins).
const DEBOUNCE_SECS: u64 = 5;
/// Remembered delivery GUIDs (GitHub redelivers on timeouts/retries).
const DEDUPE_WINDOW: usize = 256;

struct ServerState {
    cfg: Config,
    webhook_secret: String,
    bot_login: String,
    log_path: String,
    seen_deliveries: Mutex<VecDeque<String>>,
    /// One in-flight review per PR; a newer event aborts and replaces it.
    inflight: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    /// Global cap on concurrent reviews (Bedrock throttling + spend control).
    review_slots: Arc<tokio::sync::Semaphore>,
    /// Circuit breaker: (utc_day_number, reviews_started_today).
    daily: Mutex<(u64, u32)>,
    /// Per-user last @-mention forced review (cooldown).
    mention_last: Mutex<HashMap<String, Instant>>,
}

pub async fn serve(cfg: Config, port: u16) -> Result<()> {
    let webhook_secret = std::env::var("GREYBEARD_WEBHOOK_SECRET")
        .context("GREYBEARD_WEBHOOK_SECRET is required for serve mode")?;
    // Used to ignore our own comments on the issue_comment command channel.
    let bot_login =
        std::env::var("GREYBEARD_BOT_LOGIN").unwrap_or_else(|_| "greybeard-bot[bot]".to_string());
    let log_path =
        std::env::var("GREYBEARD_LOG_FILE").unwrap_or_else(|_| "greybeard-runs.jsonl".to_string());

    let slots = Arc::new(tokio::sync::Semaphore::new(cfg.max_concurrent_reviews));
    let state = Arc::new(ServerState {
        cfg,
        webhook_secret,
        bot_login,
        log_path,
        seen_deliveries: Mutex::new(VecDeque::new()),
        inflight: Mutex::new(HashMap::new()),
        review_slots: slots,
        daily: Mutex::new((0, 0)),
        mention_last: Mutex::new(HashMap::new()),
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/webhook", post(webhook))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    eprintln!("greybeard: serving on :{port}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

async fn health() -> (StatusCode, axum::Json<Value>) {
    (
        StatusCode::OK,
        axum::Json(json!({"status": "ok", "version": env!("CARGO_PKG_VERSION")})),
    )
}

async fn webhook(
    State(st): State<Arc<ServerState>>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, &'static str) {
    let Some(signature) = headers.get("x-hub-signature-256").and_then(|v| v.to_str().ok()) else {
        return (StatusCode::UNAUTHORIZED, "missing signature");
    };
    if !verify_signature(&st.webhook_secret, &body, signature) {
        return (StatusCode::UNAUTHORIZED, "bad signature");
    }
    let event = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let delivery = headers
        .get("x-github-delivery")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    {
        let mut seen = st.seen_deliveries.lock().unwrap();
        if seen.contains(&delivery) {
            return (StatusCode::ACCEPTED, "duplicate delivery");
        }
        seen.push_back(delivery);
        if seen.len() > DEDUPE_WINDOW {
            seen.pop_front();
        }
    }

    let Ok(v) = serde_json::from_slice::<Value>(&body) else {
        return (StatusCode::BAD_REQUEST, "bad json");
    };

    match event {
        "ping" => (StatusCode::OK, "pong"),
        "pull_request" => {
            let action = v["action"].as_str().unwrap_or("");
            let relevant = matches!(action, "opened" | "synchronize" | "ready_for_review" | "reopened");
            if !relevant {
                return (StatusCode::ACCEPTED, "ignored action");
            }
            // Drafts wait for ready_for_review (eligibility would skip anyway).
            if v["pull_request"]["draft"].as_bool().unwrap_or(false) {
                return (StatusCode::ACCEPTED, "draft — waiting for ready_for_review");
            }
            let Some(pr) = pr_from_payload(&v) else {
                return (StatusCode::BAD_REQUEST, "no PR coordinates");
            };
            schedule(&st, pr, action, false);
            (StatusCode::ACCEPTED, "queued")
        }
        "issue_comment" => {
            // Command channel: "@greybeard-bot review" on a PR forces a re-review.
            let is_pr = v["issue"]["pull_request"].is_object();
            let created = v["action"] == "created";
            let body_text = v["comment"]["body"].as_str().unwrap_or("").trim();
            let sender = v["comment"]["user"]["login"].as_str().unwrap_or("");
            let mention = format!("@{}", st.bot_login.trim_end_matches("[bot]"));
            if created
                && is_pr
                && sender != st.bot_login
                && body_text.starts_with(&mention)
                && body_text.contains("review")
            {
                // Per-user cooldown — the mention channel is a spend lever.
                {
                    let mut last = st.mention_last.lock().unwrap();
                    if let Some(t) = last.get(sender) {
                        if t.elapsed() < Duration::from_secs(st.cfg.mention_cooldown_secs) {
                            return (StatusCode::ACCEPTED, "rate limited — try again later");
                        }
                    }
                    last.insert(sender.to_string(), Instant::now());
                }
                let repo_full = v["repository"]["full_name"].as_str().unwrap_or("");
                let number = v["issue"]["number"].as_u64().unwrap_or(0);
                let mut parts = repo_full.split('/');
                if let (Some(owner), Some(repo)) = (parts.next(), parts.next()) {
                    if number > 0 {
                        let pr = PrRef {
                            owner: owner.to_string(),
                            repo: repo.to_string(),
                            number,
                        };
                        schedule(&st, pr, "command", true);
                        return (StatusCode::ACCEPTED, "queued (forced)");
                    }
                }
            }
            (StatusCode::ACCEPTED, "ignored comment")
        }
        _ => (StatusCode::ACCEPTED, "ignored event"),
    }
}

fn pr_from_payload(v: &Value) -> Option<PrRef> {
    Some(PrRef {
        owner: v["repository"]["owner"]["login"].as_str()?.to_string(),
        repo: v["repository"]["name"].as_str()?.to_string(),
        number: v["pull_request"]["number"].as_u64()?,
    })
}

/// Debounced, per-PR-exclusive scheduling: a newer event for the same PR
/// aborts the in-flight run and restarts against the newest head.
fn schedule(st: &Arc<ServerState>, pr: PrRef, action: &str, force: bool) {
    let key = format!("{}/{}#{}", pr.owner, pr.repo, pr.number);
    let action = action.to_string();
    let st2 = Arc::clone(st);
    let key2 = key.clone();

    let handle = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(DEBOUNCE_SECS)).await;

        // Daily circuit breaker: bounded worst-case spend no matter what the
        // webhook firehose does.
        {
            let day = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() / 86_400;
            let mut daily = st2.daily.lock().unwrap();
            if daily.0 != day {
                *daily = (day, 0);
            }
            if daily.1 >= st2.cfg.daily_review_limit {
                eprintln!("greybeard: daily review limit ({}) reached — skipping {key2}", st2.cfg.daily_review_limit);
                append_log(&st2.log_path, &json!({"pr": key2, "action": action, "outcome": "daily-limit"}));
                st2.inflight.lock().unwrap().remove(&key2);
                return;
            }
            daily.1 += 1;
        }

        // Global concurrency cap (never closed, so acquire cannot fail).
        let _permit = st2.review_slots.clone().acquire_owned().await.expect("semaphore open");
        let outcome = run_review(&st2.cfg, &pr, force).await;
        let line = json!({
            "pr": key2,
            "action": action,
            "force": force,
            "outcome": match &outcome { Ok(s) => s.as_str(), Err(_) => "error" },
            "error": outcome.as_ref().err().map(|e| e.to_string()),
        });
        eprintln!("greybeard: run {line}");
        append_log(&st2.log_path, &line);
        // Remove our entry ONLY if it is still ours — a newer event may have
        // replaced it while we were finishing, and evicting the successor's
        // handle would let two reviews of the same PR run concurrently.
        let mut inflight = st2.inflight.lock().unwrap();
        if inflight.get(&key2).map(|h| h.id()) == Some(tokio::task::id()) {
            inflight.remove(&key2);
        }
    });

    if let Some(old) = st.inflight.lock().unwrap().insert(key, handle) {
        old.abort();
    }
}

async fn run_review(cfg: &Config, pr: &PrRef, force: bool) -> Result<String> {
    let gh = Github::new().await?;
    let telemetry = Telemetry::new();
    let llm = Llm::new(cfg.clone(), telemetry.clone()).await?;
    review::run(&gh, &llm, cfg, &telemetry, pr, &ReviewArgs { dry_run: false, force }).await?;
    Ok("ok".to_string())
}

fn append_log(path: &str, line: &Value) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

/// HMAC-SHA256 (RFC 2104) over the raw body; header form "sha256=<hex>".
pub fn verify_signature(secret: &str, body: &[u8], header: &str) -> bool {
    let Some(hex_sig) = header.strip_prefix("sha256=") else {
        return false;
    };
    let Some(expected) = hex_decode(hex_sig) else {
        return false;
    };
    // verify_slice is constant-time.
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes())
        .expect("hmac accepts any key length");
    mac.update(body);
    mac.verify_slice(&expected).is_ok()
}

pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    use hmac::{Hmac, Mac};
    let mut mac =
        Hmac::<sha2::Sha256>::new_from_slice(key).expect("hmac accepts any key length");
    mac.update(msg);
    mac.finalize().into_bytes().into()
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}
