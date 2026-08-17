use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::llm::Usage;

#[derive(Debug, Clone)]
struct Row {
    label: String,
    ms: u128,
    input: u64,
    output: u64,
    cache_write: u64,
    cache_read: u64,
}

/// Per-call timing + token accounting, shared across the pipeline.
#[derive(Clone, Default)]
pub struct Telemetry {
    rows: Arc<Mutex<Vec<Row>>>,
}

impl Telemetry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, label: &str, elapsed: Duration, usage: &Usage) {
        self.rows.lock().unwrap().push(Row {
            label: label.to_string(),
            ms: elapsed.as_millis(),
            input: usage.input_tokens,
            output: usage.output_tokens,
            cache_write: usage.cache_creation_input_tokens,
            cache_read: usage.cache_read_input_tokens,
        });
    }

    /// Render the per-call table plus totals. Zero cache_read on calls after the
    /// first per tier means the pack render is nondeterministic — investigate.
    pub fn report(&self, wall: Duration) -> String {
        let rows = self.rows.lock().unwrap();
        let mut out = String::from(
            "\n  call                                ms      in     out  cache_wr  cache_rd\n",
        );
        let (mut ti, mut to, mut tw, mut tr) = (0u64, 0u64, 0u64, 0u64);
        for r in rows.iter() {
            out.push_str(&format!(
                "  {:<32} {:>7} {:>7} {:>7} {:>9} {:>9}\n",
                truncate(&r.label, 32),
                r.ms,
                r.input,
                r.output,
                r.cache_write,
                r.cache_read
            ));
            ti += r.input;
            to += r.output;
            tw += r.cache_write;
            tr += r.cache_read;
        }
        out.push_str(&format!(
            "  {:<32} {:>7} {:>7} {:>7} {:>9} {:>9}\n",
            "TOTAL",
            wall.as_millis(),
            ti,
            to,
            tw,
            tr
        ));
        out
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n - 1])
    }
}
