use crate::github::{comment, PrRef};
use crate::pipeline::{severity_rank, Confirmed};

/// Deterministic dedupe: same file + overlapping line window (±3) counts as one
/// finding; keep the highest severity (then highest confidence), credit all lenses.
pub fn dedupe(mut confirmed: Vec<Confirmed>) -> Vec<Confirmed> {
    confirmed.sort_by(|a, b| {
        severity_rank(&a.finding.severity)
            .cmp(&severity_rank(&b.finding.severity))
            .then(b.confidence.cmp(&a.confidence))
            .then(a.finding.file.cmp(&b.finding.file))
    });
    let mut kept: Vec<Confirmed> = Vec::new();
    for c in confirmed {
        let dup = kept.iter_mut().find(|k| {
            k.finding.file == c.finding.file
                && match (k.finding.line, c.finding.line) {
                    (Some(a), Some(b)) => a.abs_diff(b) <= 3,
                    (None, None) => true,
                    _ => false,
                }
        });
        match dup {
            Some(k) => {
                if !k.lens.contains(&c.lens) {
                    k.lens = format!("{}, {}", k.lens, c.lens);
                }
            }
            None => kept.push(c),
        }
    }
    kept
}

/// The one line of lore Greybeard allows itself: a closing verdict.
/// Everything above it stays plain-engineer (docs/VOICE.md).
fn verdict_line(confirmed: &[Confirmed]) -> &'static str {
    if confirmed.is_empty() {
        "You shall pass."
    } else if confirmed.iter().any(|c| c.finding.severity == "blocker") {
        "You shall not pass."
    } else {
        "Pass — but mind the cracks in the bridge."
    }
}

/// Render the single Greybeard PR comment (created once, updated in place).
pub fn render_comment(
    pr: &PrRef,
    head_sha: &str,
    confirmed: &[Confirmed],
    candidates: usize,
    lenses_run: usize,
    unverified: usize,
) -> String {
    let short_sha = &head_sha[..head_sha.len().min(7)];
    let mut out = String::from("## Greybeard review\n\n");

    if confirmed.is_empty() {
        out.push_str(
            "No issues found. Checked CI/config changes, CLAUDE.md compliance, bugs in the \
             changed code, file history, prior review feedback, and in-code guidance.\n",
        );
    } else {
        out.push_str(&format!(
            "Found {} issue{}:\n\n",
            confirmed.len(),
            if confirmed.len() == 1 { "" } else { "s" }
        ));
        for (i, c) in confirmed.iter().enumerate() {
            out.push_str(&format!(
                "{}. **[{}]** {}\n   {}\n   {}\n\n",
                i + 1,
                c.finding.severity,
                c.finding.claim.trim_end_matches('.'),
                c.finding.evidence.replace('\n', " "),
                comment::permalink(pr, head_sha, &c.finding.file, c.finding.line),
            ));
        }
    }

    if unverified > 0 {
        // A dead verifier degrades loudly: no verdict line, an explicit warning
        // instead (bake-off 2026-08-17 — never a silent "You shall pass").
        out.push_str(&format!(
            "\n**Verification degraded:** {unverified} candidate finding{} could not be \
             verified on this run and {} not shown. Treat this review as incomplete.\n",
            if unverified == 1 { "" } else { "s" },
            if unverified == 1 { "is" } else { "are" },
        ));
    } else {
        out.push_str(&format!("\n_{}_\n", verdict_line(confirmed)));
    }
    out.push_str(&format!(
        "\n<sub>Greybeard · reviewed {short_sha} · {lenses_run} lenses, {candidates} candidates, {} confirmed</sub>\n{}\n",
        confirmed.len(),
        comment::render_marker(head_sha),
    ));
    out
}
