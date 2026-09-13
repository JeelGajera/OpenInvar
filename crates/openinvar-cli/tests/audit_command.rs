//! `openinvar audit` as a gate meets it.
//!
//! An audit finding is an accusation, so the tests weighted heaviest are the
//! ones about what the command must not claim: that a clean run is a clean
//! bill of health, that a suppressed finding can vanish, or that an audit
//! which could not run is the same as one that found nothing.

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/test-edges");
    let dest = std::env::temp_dir().join(format!("openinvar-auditcmd-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dest);
    copy_tree(&src, &dest);
    dest
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create dir");
    for entry in std::fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

fn openinvar(root: &Path, args: &[&str]) -> (String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_openinvar"))
        .args(args)
        .current_dir(root)
        .output()
        .expect("run openinvar");
    (
        strip_ansi(&String::from_utf8_lossy(&output.stdout)),
        output.status.code().unwrap_or(-1),
    )
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

/// Record `HEAD`, apply `change`, record `worktree`.
fn staged(name: &str, change: impl FnOnce(&Path)) -> PathBuf {
    let root = fixture(name);
    for args in [
        vec!["init", "-q", "."],
        vec!["add", "-A"],
    ] {
        let ok = Command::new("git")
            .args(&args)
            .current_dir(&root)
            .status()
            .expect("git");
        assert!(ok.success(), "git {args:?}");
    }
    let committed = Command::new("git")
        .args([
            "-c", "user.email=t@t", "-c", "user.name=t",
            "commit", "-qm", "base",
        ])
        .current_dir(&root)
        .status()
        .expect("git commit");
    assert!(committed.success());

    let (_, code) = openinvar(&root, &["analyze", ".", "--snapshot", "HEAD"]);
    assert_eq!(code, 0, "recording HEAD");

    change(&root);

    let (_, code) = openinvar(&root, &["analyze", ".", "--snapshot", "worktree"]);
    assert_eq!(code, 0, "recording worktree");
    root
}

/// The reward-hacking shape: change the code, delete the coverage.
fn tamper(root: &Path) {
    let payload = root.join("src/payload.ts");
    let text = std::fs::read_to_string(&payload).expect("read");
    std::fs::write(
        &payload,
        text.replace("return payload.userId;", "return payload.email;"),
    )
    .expect("write");
    std::fs::write(
        root.join("src/payload.test.ts"),
        "export function testEncode() {\n  return 1;\n}\n",
    )
    .expect("write");
}

#[test]
fn tampering_is_reported_with_its_evidence_and_a_suppression_id() {
    let root = staged("tamper", tamper);
    let (out, code) = openinvar(&root, &["audit", ".", "--base", "HEAD", "--head", "worktree"]);

    assert_eq!(code, 1, "{out}");
    assert!(out.contains("test-tampering"), "{out}");
    assert!(out.contains("stopped covering"), "{out}");
    // The reader is being asked to accept an accusation, so it has to be
    // checkable rather than merely asserted.
    assert!(out.contains("changed in this diff"), "evidence missing:\n{out}");
    // And the id someone would write down, so suppression is a documented act
    // rather than something done by deleting the check.
    assert!(out.contains("suppress with:"), "{out}");
}

#[test]
fn an_ordinary_change_reports_nothing_and_still_refuses_to_call_itself_clean() {
    let root = staged("ordinary", |root| {
        let payload = root.join("src/payload.ts");
        let text = std::fs::read_to_string(&payload).expect("read");
        std::fs::write(&payload, format!("{text}\nexport const VERSION = 1;\n")).expect("write");
    });
    let (out, code) = openinvar(&root, &["audit", ".", "--base", "HEAD", "--head", "worktree"]);

    assert_eq!(code, 0, "{out}");
    assert!(out.contains("No findings"), "{out}");
    assert!(
        out.contains("not a clean bill of health"),
        "only some detectors ran, and the output must say so:\n{out}"
    );
}

#[test]
fn detectors_that_were_not_run_are_named_with_their_reasons() {
    // An absent check is otherwise indistinguishable from one that passed,
    // which is the exact shape of failure this command exists to catch.
    let root = staged("coverage", |_| {});
    let (out, _) = openinvar(&root, &["audit", ".", "--base", "HEAD", "--head", "worktree"]);

    for detector in ["assertion-removal", "scope-creep", "special-casing"] {
        assert!(out.contains(detector), "{detector} not named:\n{out}");
    }
    assert!(out.contains("not in the graph"), "the reason is missing:\n{out}");
}

#[test]
fn a_suppressed_finding_is_shown_and_stops_failing_the_gate() {
    let root = staged("suppress", tamper);
    let (out, _) = openinvar(&root, &["audit", ".", "--base", "HEAD", "--head", "worktree", "--json"]);
    let id = out
        .split(r#""id":""#)
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("a finding id")
        .to_string();

    std::fs::create_dir_all(root.join(".openinvar")).expect("mkdir");
    std::fs::write(
        root.join(".openinvar/audit-ignore"),
        format!("{id}  # rewritten for the new API\n"),
    )
    .expect("write");

    let (out, code) = openinvar(&root, &["audit", ".", "--base", "HEAD", "--head", "worktree"]);
    assert_eq!(code, 0, "a suppressed finding must not fail the gate:\n{out}");
    assert!(out.contains("Suppressed"), "{out}");
    assert!(
        out.contains("rewritten for the new API"),
        "the reason must survive into the report:\n{out}"
    );
}

#[test]
fn a_suppression_that_matches_nothing_is_reported_as_stale() {
    let root = staged("stale", |_| {});
    std::fs::create_dir_all(root.join(".openinvar")).expect("mkdir");
    std::fs::write(
        root.join(".openinvar/audit-ignore"),
        "test-tampering-deadbeef  # long since fixed\n",
    )
    .expect("write");

    let (out, _) = openinvar(&root, &["audit", ".", "--base", "HEAD", "--head", "worktree"]);
    assert!(out.contains("Stale suppressions"), "{out}");
    assert!(out.contains("test-tampering-deadbeef"), "{out}");
}

#[test]
fn a_missing_snapshot_is_an_error_rather_than_a_clean_audit() {
    let root = fixture("nosnapshot");
    let (_, code) = openinvar(&root, &["audit", ".", "--base", "HEAD", "--head", "worktree"]);
    assert_eq!(code, 2, "an audit that could not run is not a passing one");
}

#[test]
fn the_severity_floor_decides_what_fails() {
    let root = staged("severity", tamper);
    let (_, at_error) =
        openinvar(&root, &["audit", ".", "--base", "HEAD", "--head", "worktree", "--severity", "error"]);
    assert_eq!(at_error, 1);

    let (out, code) = openinvar(&root, &["audit", ".", "--base", "HEAD", "--head", "worktree", "--severity", "nonsense"]);
    assert_eq!(code, 2, "an unknown severity is refused rather than guessed:\n{out}");
}

#[test]
fn json_carries_the_findings_the_suppressions_and_what_did_not_run() {
    let root = staged("json", tamper);
    let (out, code) =
        openinvar(&root, &["audit", ".", "--base", "HEAD", "--head", "worktree", "--json"]);

    assert_eq!(code, 1);
    let line = out.trim();
    assert!(line.starts_with('{'), "expected JSON:\n{line}");
    assert!(line.contains(r#""schema_version":1"#));
    assert!(line.contains(r#""detector":"test-tampering""#));
    assert!(line.contains(r#""detectors_not_run""#), "{line}");
    assert!(line.contains(r#""exit_code":1"#));
}

#[test]
fn repeated_runs_produce_identical_output() {
    let root = staged("deterministic", tamper);
    let (first, code) =
        openinvar(&root, &["audit", ".", "--base", "HEAD", "--head", "worktree", "--json"]);
    for _ in 0..3 {
        let (again, again_code) =
            openinvar(&root, &["audit", ".", "--base", "HEAD", "--head", "worktree", "--json"]);
        assert_eq!(again, first);
        assert_eq!(again_code, code);
    }
}

#[test]
fn the_audit_hook_ships_executable_and_is_wired_into_both_gates() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let hook = repo.join("agent-configs/hooks/claude/openinvar-audit-check.sh");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&hook).expect("hook exists").permissions().mode();
        assert!(mode & 0o111 != 0, "git ignores a non-executable hook");
    }

    let settings = std::fs::read_to_string(repo.join("agent-configs/hooks/claude/settings.json"))
        .expect("settings");
    assert!(
        settings.contains("openinvar-audit-check.sh"),
        "the hook ships but nothing runs it"
    );

    let pre_commit =
        std::fs::read_to_string(repo.join("agent-configs/hooks/git/pre-commit")).expect("hook");
    assert!(pre_commit.contains("audit"), "the git gate does not run the audit");
}
