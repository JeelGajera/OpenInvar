//! `openinvar check` as CI meets it.
//!
//! The exit status is the product here: a pipeline reads it and nothing else.
//! So most of these tests assert on the status code, and the ones that assert
//! on text are the cases where the status alone would mislead — a warn-level
//! breach that exits 0, and an undecided rule that also exits 0. Both must say
//! so on stdout, because a reader who sees only "exit 0" would conclude the
//! repository is clean.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch_copy(test_name: &str) -> PathBuf {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/alias-import-bug");
    let dest =
        std::env::temp_dir().join(format!("openinvar-check-{test_name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dest);
    copy_tree(&src, &dest);

    let analyzed = Command::new(env!("CARGO_BIN_EXE_openinvar"))
        .arg("analyze")
        .arg(&dest)
        .arg("--json")
        .output()
        .expect("analyze the fixture");
    assert!(analyzed.status.success(), "fixture analysis must succeed");
    dest
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create scratch dir");
    for entry in std::fs::read_dir(from).expect("read fixture dir") {
        let entry = entry.expect("read entry");
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy file");
        }
    }
}

struct Run {
    stdout: String,
    code: i32,
}

/// Run `check` in `root`, writing `rules` to a file first when given.
fn check(root: &Path, rules: Option<&str>, extra: &[&str]) -> Run {
    let mut command = Command::new(env!("CARGO_BIN_EXE_openinvar"));
    command.arg("check").arg(root);

    if let Some(text) = rules {
        let path = root.join("rules.toml");
        std::fs::write(&path, text).expect("write rules file");
        command.arg("--rules").arg(&path);
    }
    command.args(extra);

    let output = command.output().expect("run openinvar check");
    Run {
        stdout: strip_ansi(&String::from_utf8_lossy(&output.stdout)),
        code: output.status.code().expect("process returned a status code"),
    }
}

/// The CLI colours its output; assertions are about words, not escapes.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The fixture imports `src/models/**` from `src/mappers/**`, resolved.
const FORBIDDEN: &str = r#"
[[rule]]
name = "mappers-must-not-import-models"
kind = "forbid-dependency"
from = "src/mappers/**"
to = "src/models/**"
"#;

// ── the exit status is the contract ──────────────────────────

#[test]
fn a_violated_error_rule_exits_one() {
    let root = scratch_copy("violated");
    let run = check(&root, Some(FORBIDDEN), &[]);

    assert_eq!(run.code, 1, "a broken rule must fail the gate:\n{}", run.stdout);
    assert!(run.stdout.contains("mappers-must-not-import-models"));
    assert!(run.stdout.contains("Violations"));
}

#[test]
fn a_satisfied_rule_exits_zero_and_names_the_rule() {
    // The same rule the other way round: nothing imports mappers from models.
    let root = scratch_copy("satisfied");
    let run = check(
        &root,
        Some(
            r#"
[[rule]]
name = "models-must-not-import-mappers"
kind = "forbid-dependency"
from = "src/models/**"
to = "src/mappers/**"
"#,
        ),
        &[],
    );

    assert_eq!(run.code, 0, "{}", run.stdout);
    assert!(run.stdout.contains("All rules satisfied"));
    // Listed by name, not merely counted: a reader has to be able to confirm
    // that the rule they care about is the one that passed.
    assert!(run.stdout.contains("models-must-not-import-mappers"));
}

#[test]
fn a_warn_severity_breach_exits_zero_but_never_claims_everything_passed() {
    let root = scratch_copy("warn");
    let run = check(
        &root,
        Some(&format!("{FORBIDDEN}severity = \"warn\"\n")),
        &[],
    );

    assert_eq!(run.code, 0, "warn severity must not fail the gate");
    assert!(
        !run.stdout.contains("All rules satisfied"),
        "a breach was found; saying everything passed would be false:\n{}",
        run.stdout
    );
    assert!(run.stdout.contains("warn severity"));
    assert!(run.stdout.contains("Violations"));
}

// ── a check that could not run is not a pass ─────────────────

#[test]
fn a_malformed_rules_file_exits_two_and_names_the_rule() {
    let root = scratch_copy("malformed");
    let run = check(
        &root,
        Some(
            r#"
[[rule]]
name = "typo"
kind = "forbid-dependancy"
from = "a/**"
to = "b/**"
"#,
        ),
        &[],
    );

    assert_eq!(
        run.code, 2,
        "an unreadable configuration is not a passing gate"
    );
}

#[test]
fn a_missing_rules_file_exits_zero_but_says_nothing_was_enforced() {
    let root = scratch_copy("norules");
    let run = check(&root, None, &[]);

    assert_eq!(run.code, 0);
    assert!(
        run.stdout.contains("No rules file"),
        "silence here is indistinguishable from a pass:\n{}",
        run.stdout
    );
    assert!(!run.stdout.contains("All rules satisfied"));
}

#[test]
fn require_rules_turns_a_missing_file_into_a_failure() {
    let root = scratch_copy("requirerules");
    let run = check(&root, None, &["--require-rules"]);

    assert_eq!(
        run.code, 2,
        "in CI a rules file that has gone missing must not pass"
    );
}

#[test]
fn diff_only_conflicts_with_an_explicit_revision() {
    // Two ways of saying which revisions to compare, one of which silently
    // wins, is a footgun in a file a person edits once and forgets.
    let root = scratch_copy("diffonlyconflict");
    let run = check(&root, Some(FORBIDDEN), &["--diff-only", "--base", "HEAD"]);

    assert_ne!(run.code, 0, "clap must reject the combination");
}

#[test]
fn base_without_head_is_refused_rather_than_half_evaluated() {
    let root = scratch_copy("halfrev");
    let run = check(&root, Some(FORBIDDEN), &["--base", "HEAD"]);

    assert_eq!(run.code, 2);
}

// ── rules that need a change ─────────────────────────────────

#[test]
fn a_delta_rule_without_a_delta_is_skipped_not_counted_as_passing() {
    let root = scratch_copy("skipped");
    let run = check(
        &root,
        Some(
            r#"
[[rule]]
name = "payload-is-stable"
kind = "no-field-removal"
symbol = "UserPayload"
"#,
        ),
        &[],
    );

    assert_eq!(run.code, 0);
    assert!(run.stdout.contains("Skipped"));
    assert!(
        !run.stdout.contains("All rules satisfied"),
        "the only rule was never evaluated; claiming it passed would be a lie:\n{}",
        run.stdout
    );
}

// ── machine-readable output ──────────────────────────────────

#[test]
fn json_carries_the_verdicts_and_agrees_with_the_exit_status() {
    let root = scratch_copy("json");
    let run = check(&root, Some(FORBIDDEN), &["--json"]);

    assert_eq!(run.code, 1);
    let line = run.stdout.trim();
    assert!(line.starts_with('{'), "expected JSON, got:\n{line}");
    assert!(line.contains(r#""schema_version":1"#));
    assert!(line.contains(r#""verdict":"violated""#));
    assert!(line.contains(r#""severity":"error""#));
    assert!(
        line.contains(r#""exit_code":1"#),
        "the reported code must match the process status:\n{line}"
    );
    assert!(line.contains(r#""violated":1"#));
}

#[test]
fn json_is_emitted_even_when_there_is_no_rules_file() {
    // A machine consumer must get a parseable answer in every case, including
    // the one where the tool did nothing.
    let root = scratch_copy("jsonnorules");
    let run = check(&root, None, &["--json"]);

    assert_eq!(run.code, 0);
    let line = run.stdout.trim();
    assert!(line.starts_with('{'), "expected JSON, got:\n{line}");
    assert!(line.contains(r#""status":"no-rules""#));
}

// ── determinism ──────────────────────────────────────────────

#[test]
fn repeated_runs_produce_identical_output() {
    let root = scratch_copy("deterministic");
    let first = check(&root, Some(FORBIDDEN), &["--json"]);
    for _ in 0..4 {
        let again = check(&root, Some(FORBIDDEN), &["--json"]);
        assert_eq!(again.stdout, first.stdout);
        assert_eq!(again.code, first.code);
    }
}
