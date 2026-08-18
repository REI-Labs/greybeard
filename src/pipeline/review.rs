use std::time::Instant;

use anyhow::{bail, Result};
use futures::future::join_all;

use crate::config::Config;
use crate::github::{comment, pack, Github, PrRef};
use crate::llm::{Llm, Tier};
use crate::pipeline::{compose, Confirmed, Eligibility, LensReport, Verdict};
use crate::prompts;
use crate::telemetry::Telemetry;

const MAX_FINDINGS_PER_LENS: usize = 8;

pub struct ReviewArgs {
    pub dry_run: bool,
    pub force: bool,
}

pub async fn run(
    gh: &Github,
    llm: &Llm,
    cfg: &Config,
    telemetry: &Telemetry,
    pr: &PrRef,
    args: &ReviewArgs,
) -> Result<()> {
    let wall = Instant::now();

    // ── Stage 0: context pack + deterministic eligibility ──────────────────
    let pack = pack::build(gh, pr, cfg).await?;
    eprintln!(
        "greybeard: pack built in {}ms ({} changed files, {} chars)",
        pack.fetch_ms,
        pack.changed_files.len(),
        pack.rendered.len()
    );

    if !args.force {
        if pack.state != "open" {
            bail!("PR is {} — nothing to review (use --force to override)", pack.state);
        }
        if pack.draft {
            println!("skipped: PR is a draft");
            return Ok(());
        }
        // Hard pre-model rule: bot-authored PRs (dependabot batches would
        // otherwise burn a review each). GREYBEARD_REVIEW_BOT_PRS=true or
        // --force to override.
        if pack.author_is_bot && !cfg.review_bot_prs {
            println!("skipped: bot-authored PR ({})", pack.author);
            return Ok(());
        }
        if let Some((_, sha)) = &pack.existing_comment {
            if *sha == pack.head_sha {
                println!("skipped: already reviewed {}", &pack.head_sha[..7]);
                return Ok(());
            }
        }
    }
    if pack.changed_files.is_empty() {
        println!("skipped: no changed files");
        return Ok(());
    }

    let system = prompts::system_blocks(&pack.rendered);

    // ── Stage 0b: cache warm (lens tier, prefill-only) ∥ eligibility (fast) ─
    let (_, eligibility) = tokio::join!(llm.warm(Tier::Lens, &system), async {
        llm.structured::<Eligibility>(
            Tier::Verify,
            "eligibility",
            &system,
            &prompts::eligibility_user_message(),
            &prompts::eligibility_schema(),
        )
        .await
    });
    if !args.force {
        if let Ok(e) = &eligibility {
            if e.skip {
                println!("skipped: {}", e.reason);
                return Ok(());
            }
        }
    }

    // ── Stage 1+2: lenses in parallel, each lens's findings verified as soon
    //    as that lens completes (no barrier between find and verify) ─────────
    let lens_chains = prompts::LENSES.iter().map(|lens| {
        let system = system.clone();
        async move {
            let report = llm
                .structured::<LensReport>(
                    Tier::Lens,
                    &format!("lens:{}", lens.key),
                    &system,
                    &prompts::lens_user_message(lens),
                    &prompts::findings_schema(),
                )
                .await;
            let findings = match report {
                Ok(r) => r.findings,
                Err(e) => {
                    eprintln!("greybeard: lens {} dropped: {e}", lens.key);
                    return Vec::new();
                }
            };
            let candidates: Vec<_> = findings.into_iter().take(MAX_FINDINGS_PER_LENS).collect();
            let verdicts = join_all(candidates.iter().map(|f| {
                let system = system.clone();
                let user = prompts::verify_user_message(
                    lens.key,
                    &f.file,
                    f.line,
                    &f.severity,
                    &f.claim,
                    &f.evidence,
                );
                async move {
                    llm.structured::<Verdict>(
                        Tier::Verify,
                        &format!("verify:{}", short_path(&f.file)),
                        &system,
                        &user,
                        &prompts::verdict_schema(),
                    )
                    .await
                }
            }))
            .await;
            candidates.into_iter().zip(verdicts).collect::<Vec<_>>()
        }
    });
    let results: Vec<Vec<_>> = join_all(lens_chains).await;

    let mut candidates_total = 0usize;
    let mut unverified = 0usize;
    let mut confirmed: Vec<Confirmed> = Vec::new();
    for (lens, chain) in prompts::LENSES.iter().zip(results) {
        for (finding, verdict) in chain {
            candidates_total += 1;
            match verdict {
                Ok(v) if v.real && v.confidence >= cfg.confidence_threshold => {
                    confirmed.push(Confirmed {
                        finding,
                        lens: lens.key.to_string(),
                        confidence: v.confidence,
                    });
                }
                Ok(v) => {
                    eprintln!(
                        "greybeard: rejected [{}] {} ({}): {}",
                        v.confidence, finding_summary(&finding.file, finding.line), lens.key, v.reason
                    );
                }
                // A dead verifier must degrade LOUDLY, never silently drop
                // findings (bake-off 2026-08-17: oracle#223).
                Err(e) => {
                    unverified += 1;
                    eprintln!(
                        "greybeard: UNVERIFIED {} ({}): {e}",
                        finding_summary(&finding.file, finding.line), lens.key
                    );
                }
            }
        }
    }

    // ── Stage 3: dedupe, render, post (update in place) ─────────────────────
    let confirmed = compose::dedupe(confirmed);
    let body = compose::render_comment(
        pr,
        &pack.head_sha,
        &confirmed,
        candidates_total,
        prompts::LENSES.len(),
        unverified,
    );

    if args.dry_run {
        println!("--- dry run: comment body ---\n{body}");
    } else {
        // Cheap "still open?" re-check replaces the old eligibility re-check agent.
        if !args.force && !still_open(gh, pr).await? {
            bail!("PR closed while reviewing — not posting");
        }
        let existing_id = pack.existing_comment.as_ref().map(|(id, _)| id.as_str());
        let url = comment::upsert(gh, &pack.pr_node_id, existing_id, &body).await?;
        println!("posted: {url}");
    }

    println!(
        "{} confirmed / {} candidates in {:.1}s",
        confirmed.len(),
        candidates_total,
        wall.elapsed().as_secs_f32()
    );
    print!("{}", telemetry.report(wall.elapsed()));
    Ok(())
}

async fn still_open(gh: &Github, pr: &PrRef) -> Result<bool> {
    const QUERY: &str = r#"
query($owner:String!,$repo:String!,$number:Int!){
  repository(owner:$owner,name:$repo){ pullRequest(number:$number){ state } }
}"#;
    let data = gh
        .graphql(
            QUERY,
            serde_json::json!({"owner": pr.owner, "repo": pr.repo, "number": pr.number}),
        )
        .await?;
    Ok(data["repository"]["pullRequest"]["state"] == "OPEN")
}

fn short_path(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn finding_summary(file: &str, line: Option<u32>) -> String {
    match line {
        Some(l) => format!("{file}:{l}"),
        None => file.to_string(),
    }
}
