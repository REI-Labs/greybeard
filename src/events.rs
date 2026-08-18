use serde_json::{json, Value};

use crate::pipeline::review::RunSummary;

/// One single-line JSON event to stdout per finished run. Promtail ships pod
/// stdout to Loki (S3-backed, 30-day retention), so this line IS the durable
/// run log; the local JSONL file is only a same-pod convenience copy.
pub struct RunEvent<'a> {
    pub pr: &'a str,
    pub action: &'a str,
    pub force: bool,
    /// posted | dry-run | skipped | daily-limit | error
    pub outcome: &'a str,
    pub error: Option<&'a str>,
    pub duration_s: f64,
    pub summary: Option<&'a RunSummary>,
    pub cost_usd: Option<f64>,
}

pub fn render(ev: &RunEvent) -> Value {
    json!({
        "evt": "review.done",
        "pr": ev.pr,
        "action": ev.action,
        "force": ev.force,
        "outcome": ev.outcome,
        "error": ev.error,
        "reason": ev.summary.and_then(|s| s.reason.as_deref()),
        "duration_s": (ev.duration_s * 10.0).round() / 10.0,
        "confirmed": ev.summary.map(|s| s.confirmed),
        "minor": ev.summary.map(|s| s.minor),
        "candidates": ev.summary.map(|s| s.candidates),
        "unverified": ev.summary.map(|s| s.unverified),
        "lenses_failed": ev.summary.map(|s| s.lenses_failed),
        "tokens": ev.summary.map(|s| json!({
            "input": s.usage.input_tokens,
            "output": s.usage.output_tokens,
            "cache_read": s.usage.cache_read_input_tokens,
            "cache_write": s.usage.cache_creation_input_tokens,
        })),
        "cost_usd": ev.cost_usd,
    })
}

pub fn emit(ev: &RunEvent) {
    // stdout, one line — the shape Promtail/Loki index. Never eprintln here.
    println!("{}", render(ev));
}
