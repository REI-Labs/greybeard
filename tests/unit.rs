use greybeard::github::comment::{parse_marker, permalink, render_marker};
use greybeard::github::pack::{cap_lines, parse_diff_ranges};
use greybeard::github::PrRef;
use greybeard::llm::parse_json_object;
use greybeard::pipeline::compose::{dedupe, render_comment};
use greybeard::pipeline::{Confirmed, Finding, LensReport};

fn pr() -> PrRef {
    PrRef::parse("https://github.com/REI-Labs/nexus-core/pull/123").unwrap()
}

fn finding(file: &str, line: Option<u32>, severity: &str) -> Finding {
    Finding {
        file: file.into(),
        line,
        claim: "claim".into(),
        severity: severity.into(),
        evidence: "evidence".into(),
    }
}

#[test]
fn pr_url_parses_and_rejects() {
    let p = pr();
    assert_eq!((p.owner.as_str(), p.repo.as_str(), p.number), ("REI-Labs", "nexus-core", 123));
    assert!(PrRef::parse("https://github.com/REI-Labs/nexus-core/issues/9").is_err());
    assert!(PrRef::parse("https://gitlab.com/x/y/pull/1").is_err());
}

#[test]
fn marker_roundtrip() {
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let marker = render_marker(sha);
    assert!(marker.starts_with("<!-- greybeard:"));
    assert_eq!(parse_marker(&format!("## review\nbody text\n{marker}\n")), Some(sha.to_string()));
    assert_eq!(parse_marker("no marker here"), None);
}

#[test]
fn permalink_full_sha_with_context_lines() {
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let link = permalink(&pr(), sha, "backend/api/foo.py", Some(42));
    assert_eq!(
        link,
        format!("https://github.com/REI-Labs/nexus-core/blob/{sha}/backend/api/foo.py#L41-L43")
    );
    // Line 1 must not produce L0.
    assert!(permalink(&pr(), sha, "f.py", Some(1)).ends_with("#L1-L2"));
    // No line → plain file link.
    assert!(!permalink(&pr(), sha, "f.py", None).contains('#'));
}

#[test]
fn diff_ranges_parse_new_side_hunks() {
    let diff = "\
diff --git a/src/a.rs b/src/a.rs
--- a/src/a.rs
+++ b/src/a.rs
@@ -10,3 +12,4 @@ fn x() {
 ctx
+added
 ctx
@@ -30,2 +40,2 @@
 ctx
diff --git a/gone.rs b/gone.rs
--- a/gone.rs
+++ /dev/null
@@ -1,5 +0,0 @@
";
    let ranges = parse_diff_ranges(diff);
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].0, "src/a.rs");
    assert_eq!(ranges[0].1, vec![(12, 15), (40, 41)]);
}

#[test]
fn cap_lines_truncates_with_note() {
    let text = (1..=10).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n");
    assert_eq!(cap_lines(&text, 20), text);
    let capped = cap_lines(&text, 3);
    assert!(capped.contains("line3"));
    assert!(!capped.contains("line4\n"));
    assert!(capped.contains("truncated: 3 of 10"));
}

#[test]
fn json_extraction_tolerates_fences_and_prose() {
    let messy = "Here you go:\n```json\n{\"findings\": [{\"file\": \"a.py\", \"claim\": \"c\", \"severity\": \"gap\", \"evidence\": \"e\"}]}\n```\nDone.";
    let report: LensReport = parse_json_object(messy).unwrap();
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].line, None);
    assert!(parse_json_object::<LensReport>("no json at all").is_err());
}

#[test]
fn dedupe_merges_nearby_and_keeps_highest_severity() {
    let confirmed = vec![
        Confirmed { finding: finding("a.py", Some(10), "nit"), lens: "bugs".into(), confidence: 85 },
        Confirmed { finding: finding("a.py", Some(12), "blocker"), lens: "history".into(), confidence: 90 },
        Confirmed { finding: finding("a.py", Some(50), "gap"), lens: "bugs".into(), confidence: 88 },
        Confirmed { finding: finding("b.py", Some(12), "gap"), lens: "bugs".into(), confidence: 88 },
    ];
    let kept = dedupe(confirmed);
    assert_eq!(kept.len(), 3);
    // The a.py:10/12 pair collapses into the blocker, crediting both lenses.
    let merged = kept.iter().find(|c| c.finding.file == "a.py" && c.finding.severity == "blocker").unwrap();
    assert!(merged.lens.contains("history") && merged.lens.contains("bugs"));
}

#[test]
fn comment_render_no_findings_and_marker() {
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let body = render_comment(&pr(), sha, &[], 4, 5);
    assert!(body.contains("No issues found"));
    assert!(body.contains("_You shall pass._"));
    assert!(body.contains("reviewed 0123456"));
    assert_eq!(parse_marker(&body), Some(sha.to_string()));
    // No AI attribution: referencing the CLAUDE.md *file* is fine, but no
    // generated-with footers, bot emoji, or vendor credits.
    let lower = body.to_lowercase();
    for banned in ["generated with", "claude code", "anthropic", "🤖"] {
        assert!(!lower.contains(banned), "attribution marker found: {banned}");
    }
}

#[test]
fn comment_render_findings_are_numbered_with_permalinks() {
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let confirmed = vec![Confirmed {
        finding: finding("src/x.rs", Some(7), "blocker"),
        lens: "bugs".into(),
        confidence: 95,
    }];
    let body = render_comment(&pr(), sha, &confirmed, 3, 5);
    assert!(body.contains("Found 1 issue:"));
    assert!(body.contains(&format!("blob/{sha}/src/x.rs#L6-L8")));
    assert!(body.contains("1 confirmed"));
    assert!(body.contains("_You shall not pass._"), "blocker gets the hard verdict line");
}

#[test]
fn verdict_line_severity_tiers() {
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let gap_only = vec![Confirmed {
        finding: finding("src/x.rs", Some(7), "gap"),
        lens: "bugs".into(),
        confidence: 90,
    }];
    let body = render_comment(&pr(), sha, &gap_only, 1, 5);
    assert!(body.contains("_Pass — but mind the cracks in the bridge._"));
    assert!(!body.contains("_You shall not pass._"));
}
