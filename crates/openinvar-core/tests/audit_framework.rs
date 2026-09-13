//! The audit framework's invariants.
//!
//! An audit finding is an accusation: it says a change looks like it was made
//! to pass a check rather than to work, and it will sometimes be levelled at
//! someone who did nothing wrong. Three properties keep that survivable, and
//! they are what these tests are about.
//!
//! An id that survives unrelated edits, so CI can say "you already looked at
//! this" instead of re-accusing on every push. A scope gate that keeps
//! accusations off evidence too weak to support them. And suppression that is
//! visible — a gate which silently drops what it was told to ignore is exactly
//! the thing this feature exists to catch.

use std::collections::BTreeSet;

use openinvar_core::audit::{
    self, AuditContext, Confidence, Detector, Evidence, Finding, FindingId, Suppressions,
};
use openinvar_core::delta::GraphDelta;
use openinvar_core::graph::InvarGraph;
use openinvar_core::rules::Severity;

// ── stable ids ───────────────────────────────────────────────

#[test]
fn an_id_depends_only_on_what_the_finding_is_about() {
    let a = FindingId::new("test-tampering", &["src/a.ts::f::function"]);
    let b = FindingId::new("test-tampering", &["src/a.ts::f::function"]);
    assert_eq!(a, b, "the same finding must keep its id between runs");

    let other = FindingId::new("test-tampering", &["src/a.ts::g::function"]);
    assert_ne!(a, other, "different subjects must not collide");

    let other_detector = FindingId::new("assertion-removal", &["src/a.ts::f::function"]);
    assert_ne!(a, other_detector);
}

#[test]
fn subject_order_does_not_change_the_id() {
    // A detector that collects its subjects in a different order on a later
    // run must not invalidate a suppression written against the first.
    let one = FindingId::new("d", &["b", "a", "c"]);
    let two = FindingId::new("d", &["c", "a", "b"]);
    assert_eq!(one, two);
}

#[test]
fn adjacent_subjects_cannot_be_confused_for_one_another() {
    // Without a separator, ("ab","c") and ("a","bc") hash the same bytes.
    assert_ne!(FindingId::new("d", &["ab", "c"]), FindingId::new("d", &["a", "bc"]));
}

#[test]
fn an_id_is_readable_and_names_its_detector() {
    // It ends up in a checked-in file that a human edits, so it has to be
    // possible to tell what a line is for without running anything.
    let id = FindingId::new("test-tampering", &["src/a.ts::f::function"]);
    assert!(id.as_str().starts_with("test-tampering-"), "{id}");
    assert_eq!(id.as_str().len(), "test-tampering-".len() + 8);
}

// ── a detector for the framework to run ──────────────────────

struct Fake {
    name: &'static str,
    severity: Severity,
    findings: Vec<Finding>,
}

impl Detector for Fake {
    fn name(&self) -> &'static str {
        self.name
    }
    fn severity(&self) -> Severity {
        self.severity
    }
    fn confidence(&self) -> Confidence {
        Confidence::High
    }
    fn describes(&self) -> &'static str {
        "a detector that reports whatever it was given"
    }
    fn detect(&self, _context: &AuditContext<'_>) -> Vec<Finding> {
        self.findings.clone()
    }
}

fn finding(detector: &'static str, file: &str, severity: Severity) -> Finding {
    Finding {
        id: FindingId::new(detector, &[file]),
        detector,
        severity,
        confidence: Confidence::High,
        summary: format!("something about {file}"),
        file: file.to_string(),
        line: 1,
        evidence: vec![Evidence {
            file: file.to_string(),
            line: 1,
            detail: "because".to_string(),
        }],
    }
}

fn context<'a>(
    before: &'a InvarGraph,
    after: &'a InvarGraph,
    delta: &'a GraphDelta,
    tier_one: &'a BTreeSet<String>,
) -> AuditContext<'a> {
    AuditContext {
        before,
        after,
        delta,
        tier_one_files: tier_one,
    }
}

fn tier_one(files: &[&str]) -> BTreeSet<String> {
    files.iter().map(|f| f.to_string()).collect()
}

// ── the scope gate ───────────────────────────────────────────

#[test]
fn a_finding_about_a_file_outside_tier_one_is_dropped() {
    // The gate that keeps an accusation off structural evidence. Enforced by
    // the framework as well as trusted to the detector, because a detector
    // that forgot to check would accuse someone on evidence that cannot see
    // across files — and that must not be one mistake away.
    let (before, after, delta) = (InvarGraph::new(), InvarGraph::new(), GraphDelta::default());
    let scope = tier_one(&["src/resolved.ts"]);
    let ctx = context(&before, &after, &delta, &scope);

    let detector = Fake {
        name: "fake",
        severity: Severity::Error,
        findings: vec![
            finding("fake", "src/resolved.ts", Severity::Error),
            finding("fake", "vendor/structural.rb", Severity::Error),
        ],
    };

    let report = audit::run(&[&detector], &ctx, &Suppressions::default());

    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].file, "src/resolved.ts");
}

// ── suppression ──────────────────────────────────────────────

#[test]
fn a_suppressed_finding_is_reported_as_suppressed_and_never_dropped() {
    // A gate that silently drops what it was told to ignore is the thing this
    // feature exists to catch.
    let (before, after, delta) = (InvarGraph::new(), InvarGraph::new(), GraphDelta::default());
    let scope = tier_one(&["src/a.ts"]);
    let ctx = context(&before, &after, &delta, &scope);

    let target = finding("fake", "src/a.ts", Severity::Error);
    let id = target.id.clone();
    let detector = Fake {
        name: "fake",
        severity: Severity::Error,
        findings: vec![target],
    };

    let suppressions = Suppressions::parse(&format!("{id}  # rewritten deliberately\n"));
    let report = audit::run(&[&detector], &ctx, &suppressions);

    assert!(report.findings.is_empty(), "it was suppressed");
    assert_eq!(report.suppressed.len(), 1);
    assert_eq!(report.suppressed[0].reason, "rewritten deliberately");
    assert!(!report.should_fail(), "a suppressed finding must not fail a gate");
}

#[test]
fn a_suppression_matching_nothing_is_reported_as_stale() {
    // Dead configuration that hides nothing while someone reading the file
    // believes a finding is being held back.
    let (before, after, delta) = (InvarGraph::new(), InvarGraph::new(), GraphDelta::default());
    let scope = tier_one(&["src/a.ts"]);
    let ctx = context(&before, &after, &delta, &scope);

    let detector = Fake {
        name: "fake",
        severity: Severity::Error,
        findings: vec![],
    };
    let suppressions = Suppressions::parse("fake-deadbeef # long since fixed\n");
    let report = audit::run(&[&detector], &ctx, &suppressions);

    assert_eq!(report.stale_suppressions, vec!["fake-deadbeef".to_string()]);
}

#[test]
fn the_ignore_file_accepts_comments_blank_lines_and_bare_ids() {
    let parsed = Suppressions::parse(
        "\n\
         # a whole-line comment\n\
         \n\
         alpha-00000001  # with a reason\n\
         beta-00000002\n\
         \n",
    );

    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed.reason_for_id("alpha-00000001"), Some("with a reason"));
    // A bare id is a valid suppression; the reason is merely absent.
    assert_eq!(parsed.reason_for_id("beta-00000002"), Some(""));
    assert_eq!(parsed.reason_for_id("never-written"), None);

    let ids: Vec<&str> = parsed.ids().collect();
    assert_eq!(ids, vec!["alpha-00000001", "beta-00000002"]);
}

// ── ordering and gating ──────────────────────────────────────

#[test]
fn findings_are_ordered_worst_first() {
    let (before, after, delta) = (InvarGraph::new(), InvarGraph::new(), GraphDelta::default());
    let scope = tier_one(&["src/a.ts", "src/b.ts"]);
    let ctx = context(&before, &after, &delta, &scope);

    let detector = Fake {
        name: "fake",
        severity: Severity::Error,
        findings: vec![
            finding("fake", "src/a.ts", Severity::Warn),
            finding("fake", "src/b.ts", Severity::Error),
        ],
    };
    let report = audit::run(&[&detector], &ctx, &Suppressions::default());

    assert_eq!(report.findings[0].severity, Severity::Error);
    assert!(report.should_fail());
    assert_eq!(report.count_of(Severity::Warn), 1);
}

#[test]
fn a_warn_only_report_does_not_fail_a_gate() {
    let (before, after, delta) = (InvarGraph::new(), InvarGraph::new(), GraphDelta::default());
    let scope = tier_one(&["src/a.ts"]);
    let ctx = context(&before, &after, &delta, &scope);

    let detector = Fake {
        name: "fake",
        severity: Severity::Warn,
        findings: vec![finding("fake", "src/a.ts", Severity::Warn)],
    };
    let report = audit::run(&[&detector], &ctx, &Suppressions::default());

    assert_eq!(report.findings.len(), 1);
    assert!(!report.should_fail());
}

#[test]
fn the_same_finding_raised_twice_is_reported_once() {
    let (before, after, delta) = (InvarGraph::new(), InvarGraph::new(), GraphDelta::default());
    let scope = tier_one(&["src/a.ts"]);
    let ctx = context(&before, &after, &delta, &scope);

    let one = Fake {
        name: "fake",
        severity: Severity::Error,
        findings: vec![finding("fake", "src/a.ts", Severity::Error)],
    };
    let two = Fake {
        name: "fake",
        severity: Severity::Error,
        findings: vec![finding("fake", "src/a.ts", Severity::Error)],
    };
    let report = audit::run(&[&one, &two], &ctx, &Suppressions::default());

    assert_eq!(report.findings.len(), 1);
}

#[test]
fn a_report_is_reproducible() {
    let (before, after, delta) = (InvarGraph::new(), InvarGraph::new(), GraphDelta::default());
    let scope = tier_one(&["src/a.ts", "src/b.ts"]);
    let ctx = context(&before, &after, &delta, &scope);

    let detector = Fake {
        name: "fake",
        severity: Severity::Error,
        findings: vec![
            finding("fake", "src/b.ts", Severity::Error),
            finding("fake", "src/a.ts", Severity::Error),
        ],
    };
    let first = audit::run(&[&detector], &ctx, &Suppressions::default());
    for _ in 0..5 {
        assert_eq!(audit::run(&[&detector], &ctx, &Suppressions::default()), first);
    }
}

#[test]
fn a_missing_ignore_file_suppresses_nothing_rather_than_erroring() {
    let path = std::env::temp_dir().join("openinvar-no-such-audit-ignore");
    let _ = std::fs::remove_file(&path);
    let loaded = Suppressions::load(&path).expect("absence is the normal case");
    assert!(loaded.is_empty());
}
