//! The shipped hook scripts, executed.
//!
//! These are templates users copy into their own repositories, so a bug here
//! ships to everyone and shows up as an agent that silently stops warning, or
//! a repository where every commit is rejected. Both happened during
//! development and neither was visible from Rust:
//!
//! * The pre-edit hook counted `"file"` keys to report dependent files, which
//!   also counted one per edge and inflated the number roughly five-fold.
//! * Sourcing the shared library with `. lib || fallback` terminated the shell
//!   when the library was absent, because a failed `.` is a special-builtin
//!   failure in POSIX sh. The pre-commit hook then rejected every commit with
//!   no message at all.
//!
//! So the counts are asserted against `impact --json` rather than against a
//! literal, and every degradation path is asserted to exit 0.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scratch_copy(test_name: &str) -> PathBuf {
    let src = repo_root().join("fixtures/alias-import-bug");
    let dest =
        std::env::temp_dir().join(format!("openinvar-hooks-{test_name}-{}", std::process::id()));
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

/// Run a shipped hook script with `payload` on stdin, from `cwd`.
fn run_hook(script: &str, cwd: &Path, payload: &str) -> Run {
    let script_path = repo_root().join("agent-configs/hooks").join(script);
    let mut child = Command::new("sh")
        .arg(&script_path)
        .current_dir(cwd)
        .env("OPENINVAR_BIN", env!("CARGO_BIN_EXE_openinvar"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hook");

    use std::io::Write;
    child
        .stdin
        .as_mut()
        .expect("hook stdin")
        .write_all(payload.as_bytes())
        .expect("write payload");

    let output = child.wait_with_output().expect("hook finished");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        code: output.status.code().unwrap_or(-1),
    }
}

fn pre_edit_payload(file: &Path) -> String {
    format!(
        r#"{{"hook_event_name":"PreToolUse","tool_name":"Edit","tool_input":{{"file_path":"{}"}}}}"#,
        file.display()
    )
}

/// The counts `impact --json` reports for a file.
fn impact_counts(root: &Path, file: &Path) -> (usize, usize) {
    let output = Command::new(env!("CARGO_BIN_EXE_openinvar"))
        .arg("impact")
        .arg(file)
        .arg("--path")
        .arg(root)
        .arg("--json")
        .output()
        .expect("run impact");
    let json = String::from_utf8_lossy(&output.stdout).to_string();

    let field = |key: &str| -> usize {
        let needle = format!("\"{key}\":");
        let start = json.find(&needle).expect("field present") + needle.len();
        json[start..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .expect("numeric field")
    };
    (field("dependent_file_count"), field("edge_count"))
}

#[test]
fn the_pre_edit_hook_reports_the_same_counts_the_cli_does() {
    // The regression that matters: a hook that under-reports blast radius is
    // worse than no hook, because the agent acts on the smaller number.
    let root = scratch_copy("counts");
    let target = root.join("src/models/user_payload.ts");
    let (files, edges) = impact_counts(&root, &target);
    assert!(files > 0, "the fixture's model must have dependents");

    let run = run_hook("claude/openinvar-pre-edit.sh", &root, &pre_edit_payload(&target));

    assert_eq!(run.code, 0);
    assert!(
        run.stdout.contains(&format!("{files} file(s)")),
        "hook must report the {files} files the CLI reports:\n{}",
        run.stdout
    );
    assert!(
        run.stdout.contains(&format!("{edges} reference(s)")),
        "hook must report the {edges} edges the CLI reports:\n{}",
        run.stdout
    );
    assert!(run.stdout.contains(r#""hookEventName":"PreToolUse""#));
}

#[test]
fn the_pre_edit_hook_says_nothing_about_a_file_the_graph_does_not_track() {
    let root = scratch_copy("untracked");
    let target = root.join("README.md");
    let run = run_hook("claude/openinvar-pre-edit.sh", &root, &pre_edit_payload(&target));

    assert_eq!(run.code, 0, "an untracked path is not an error");
    assert!(
        run.stdout.trim().is_empty(),
        "silence costs the agent nothing; a paragraph about a README costs context:\n{}",
        run.stdout
    );
}

#[test]
fn the_pre_edit_hook_ignores_a_payload_with_no_file() {
    let root = scratch_copy("nofile");
    let run = run_hook(
        "claude/openinvar-pre-edit.sh",
        &root,
        r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"ls"}}"#,
    );

    assert_eq!(run.code, 0);
    assert!(run.stdout.trim().is_empty());
}

#[test]
fn the_pre_edit_hook_is_silent_when_openinvar_is_not_installed() {
    // A repository is shared with people who have not installed OpenInvar.
    let root = scratch_copy("nobin");
    let target = root.join("src/models/user_payload.ts");
    let script = repo_root().join("agent-configs/hooks/claude/openinvar-pre-edit.sh");

    let mut child = Command::new("sh")
        .arg(&script)
        .current_dir(&root)
        .env("OPENINVAR_BIN", "/nonexistent/openinvar")
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn hook");
    use std::io::Write;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(pre_edit_payload(&target).as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();

    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());
}

#[test]
fn a_hook_whose_library_is_missing_exits_zero_rather_than_blocking() {
    // `. missing_file || fallback` terminates a POSIX shell outright. When
    // that happened the pre-commit hook exited non-zero with no output and
    // rejected every commit in the repository.
    let root = scratch_copy("nolib");
    let isolated = root.join("hooks-without-lib");
    std::fs::create_dir_all(&isolated).expect("create dir");

    for script in [
        "claude/openinvar-pre-edit.sh",
        "claude/openinvar-post-edit.sh",
        "claude/openinvar-stop-check.sh",
        "git/pre-commit",
        "git/post-commit",
    ] {
        let source = repo_root().join("agent-configs/hooks").join(script);
        let name = Path::new(script).file_name().unwrap();
        let copied = isolated.join(name);
        std::fs::copy(&source, &copied).expect("copy script");

        let output = Command::new("sh")
            .arg(&copied)
            .current_dir(&root)
            .env("OPENINVAR_BIN", env!("CARGO_BIN_EXE_openinvar"))
            .stdin(Stdio::null())
            .output()
            .expect("run hook");

        assert_eq!(
            output.status.code(),
            Some(0),
            "{script} must exit 0 without its library, not block:\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn every_shipped_hook_is_executable_and_starts_with_a_shebang() {
    use std::os::unix::fs::PermissionsExt;

    for script in [
        "claude/openinvar-pre-edit.sh",
        "claude/openinvar-post-edit.sh",
        "claude/openinvar-stop-check.sh",
        "git/pre-commit",
        "git/post-commit",
        "lib/openinvar-hook-lib.sh",
    ] {
        let path = repo_root().join("agent-configs/hooks").join(script);
        let mode = std::fs::metadata(&path)
            .expect("hook exists")
            .permissions()
            .mode();
        assert!(
            mode & 0o111 != 0,
            "{script} must be committed executable; git preserves the bit and a \
             copied non-executable hook is one git silently ignores"
        );
        let text = std::fs::read_to_string(&path).expect("read hook");
        assert!(text.starts_with("#!/bin/sh"), "{script} must be POSIX sh");
    }
}
