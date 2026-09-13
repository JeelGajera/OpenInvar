//! `openinvar tests` as a verify loop meets it.
//!
//! The exit status is the product. An agent runs the selected tests when it is
//! 0 and the whole suite otherwise, so these assert the status far more than
//! they assert the prose — and most of them assert the case where the status
//! must refuse to say "complete".

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(test_name: &str) -> PathBuf {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/test-edges");
    let dest =
        std::env::temp_dir().join(format!("openinvar-testsel-{test_name}-{}", std::process::id()));
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

fn tests(root: &Path, args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_openinvar"))
        .arg("tests")
        .args(args)
        .arg("--path")
        .arg(root)
        .output()
        .expect("run openinvar tests");
    Run {
        stdout: strip_ansi(&String::from_utf8_lossy(&output.stdout)),
        code: output.status.code().expect("status code"),
    }
}

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

#[test]
fn a_covered_symbol_selects_its_test_and_reports_a_complete_selection() {
    let root = fixture("covered");
    let run = tests(&root, &["encode"]);

    assert_eq!(run.code, 0, "a fully covered symbol is complete:\n{}", run.stdout);
    assert!(run.stdout.contains("payload.test.ts"), "{}", run.stdout);
    assert!(run.stdout.contains("may be run in place of the full suite"));
}

#[test]
fn a_symbol_no_test_reaches_never_reports_a_complete_selection() {
    // The failure this command exists to prevent: naming zero tests and
    // letting a caller read that as "nothing needs running".
    let root = fixture("uncovered");
    std::fs::write(
        root.join("src/orphan.ts"),
        "export function neverTested(): number {\n  return 42;\n}\n",
    )
    .expect("write orphan");
    let analyzed = Command::new(env!("CARGO_BIN_EXE_openinvar"))
        .arg("analyze")
        .arg(&root)
        .arg("--json")
        .output()
        .expect("re-analyze");
    assert!(analyzed.status.success());

    let run = tests(&root, &["neverTested"]);

    assert_eq!(
        run.code, 3,
        "an uncovered symbol must not report a complete selection:\n{}",
        run.stdout
    );
    assert!(run.stdout.contains("may be incomplete"));
    assert!(run.stdout.contains("Run the full suite"));
    assert!(!run.stdout.contains("may be run in place of the full suite"));
}

#[test]
fn an_unknown_symbol_is_an_error_rather_than_an_empty_selection() {
    // An empty selection and an unrecognised name look identical to a script.
    let root = fixture("unknown");
    let run = tests(&root, &["definitelyNotASymbol"]);

    assert_eq!(run.code, 2, "{}", run.stdout);
}

#[test]
fn diff_without_a_recorded_snapshot_is_an_error_not_a_pass() {
    let root = fixture("nosnapshot");
    let run = tests(&root, &["--diff", "--base", "HEAD", "--head", "worktree"]);

    assert_eq!(run.code, 2, "{}", run.stdout);
}

#[test]
fn neither_a_symbol_nor_diff_is_refused() {
    let root = fixture("noargs");
    let run = tests(&root, &[]);

    assert_eq!(run.code, 2);
}

#[test]
fn json_carries_the_gaps_and_agrees_with_the_exit_status() {
    let root = fixture("json");
    let run = tests(&root, &["encode", "--json"]);

    assert_eq!(run.code, 0);
    let line = run.stdout.trim();
    assert!(line.starts_with('{'), "expected JSON:\n{line}");
    assert!(line.contains(r#""schema_version":1"#));
    assert!(line.contains(r#""complete":true"#));
    assert!(line.contains(r#""exit_code":0"#));
    assert!(line.contains(r#""gaps":[]"#));
    assert!(line.contains("payload.test.ts"));
}

#[test]
fn repeated_runs_produce_identical_output() {
    let root = fixture("deterministic");
    let first = tests(&root, &["encode", "--json"]);
    for _ in 0..3 {
        let again = tests(&root, &["encode", "--json"]);
        assert_eq!(again.stdout, first.stdout);
        assert_eq!(again.code, first.code);
    }
}
