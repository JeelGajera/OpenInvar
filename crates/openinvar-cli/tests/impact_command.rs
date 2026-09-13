//! `openinvar impact` — the file-level question a pre-edit hook asks.
//!
//! Written for a caller that cannot recover from an error: a hook knows a path
//! and nothing else, and the path is often one the graph has never seen. The
//! tests that matter are therefore about what happens on unremarkable input,
//! not about the happy path.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch_copy(test_name: &str) -> PathBuf {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/alias-import-bug");
    let dest =
        std::env::temp_dir().join(format!("openinvar-impact-{test_name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dest);
    copy_tree(&src, &dest);

    let analyzed = Command::new(env!("CARGO_BIN_EXE_openinvar"))
        .arg("analyze")
        .arg(&dest)
        .arg("--json")
        .output()
        .expect("analyze the fixture");
    assert!(analyzed.status.success());
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
    success: bool,
}

fn impact(root: &Path, file: &str, extra: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_openinvar"))
        .arg("impact")
        .arg(file)
        .arg("--path")
        .arg(root)
        .args(extra)
        .output()
        .expect("run impact");
    Run {
        stdout: strip_ansi(&String::from_utf8_lossy(&output.stdout)),
        success: output.status.success(),
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
fn a_file_other_files_import_reports_them() {
    let root = scratch_copy("found");
    let run = impact(&root, "src/models/user_payload.ts", &[]);

    assert!(run.success);
    assert!(
        run.stdout.contains("view_model_mapper.ts"),
        "the mapper imports this model:\n{}",
        run.stdout
    );
}

#[test]
fn an_absolute_path_resolves_the_same_as_a_relative_one() {
    // A hook is handed an absolute path by the agent that invoked it; the
    // graph stores repository-relative paths. Getting this wrong does not
    // error, it silently finds nothing — the worst possible failure here.
    let root = scratch_copy("absolute");
    let relative = impact(&root, "src/models/user_payload.ts", &["--json"]);
    let absolute = impact(
        &root,
        root.join("src/models/user_payload.ts").to_str().unwrap(),
        &["--json"],
    );

    assert!(relative.success && absolute.success);
    assert_eq!(relative.stdout, absolute.stdout);
}

#[test]
fn a_file_the_graph_does_not_track_is_reported_not_refused() {
    let root = scratch_copy("untracked");
    let run = impact(&root, "README.md", &[]);

    assert!(
        run.success,
        "a hook that errors on the first untracked file gets removed"
    );
    assert!(run.stdout.contains("none recorded"));
}

#[test]
fn a_path_that_does_not_exist_at_all_is_still_not_an_error() {
    let root = scratch_copy("missing");
    let run = impact(&root, "src/does/not/exist.ts", &[]);

    assert!(run.success);
    assert!(run.stdout.contains("none recorded"));
}

#[test]
fn json_reports_counts_as_fields_rather_than_leaving_them_to_be_derived() {
    // A shell hook without `jq` counts key occurrences, and "file" appears
    // once per edge as well as once per dependent. Deriving the count that way
    // over-reported it five-fold until these fields existed.
    let root = scratch_copy("jsonfields");
    let run = impact(&root, "src/models/user_payload.ts", &["--json"]);

    assert!(run.success);
    let line = run.stdout.trim();
    assert!(line.starts_with('{'), "expected JSON:\n{line}");
    assert!(line.contains(r#""schema_version":1"#));
    assert!(line.contains(r#""dependent_file_count":"#));
    assert!(line.contains(r#""edge_count":"#));
    assert!(line.contains(r#""gate_safe":"#));
}

#[test]
fn repeated_runs_produce_identical_output() {
    let root = scratch_copy("deterministic");
    let first = impact(&root, "src/models/user_payload.ts", &["--json"]);
    for _ in 0..4 {
        assert_eq!(
            impact(&root, "src/models/user_payload.ts", &["--json"]).stdout,
            first.stdout
        );
    }
}
