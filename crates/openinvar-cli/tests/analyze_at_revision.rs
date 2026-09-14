//! `openinvar analyze --at <rev>`.
//!
//! `--at` reads a revision's tree out of git instead of the working tree, which
//! introduces two things the rest of `analyze` never had to worry about: a
//! second directory that the result must not mention, and uncommitted work that
//! the command must not disturb. Both are tested here, because both fail
//! silently — a leaked temporary path looks like an ordinary run until two runs
//! are compared, and a clobbered working tree is noticed long after the command
//! that did it.
//!
//! These drive the real binary. A temporary worktree is a property of the
//! process and its cleanup, not of a library call.

use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_openinvar")
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn openinvar(root: &Path, args: &[&str]) -> (String, i32) {
    let output = Command::new(binary())
        .args(args)
        .current_dir(root)
        .output()
        .expect("run openinvar");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

/// A two-commit repository: `alpha.ts` alone, then `beta.ts` added.
///
/// Two commits is the minimum that can tell "analysed the revision" apart from
/// "analysed the working tree and labelled it with a revision" — the failure
/// this whole feature has to avoid, and one that a single-commit fixture would
/// pass while broken.
struct Repo {
    root: PathBuf,
    first: String,
}

impl Repo {
    fn create(name: &str) -> Self {
        // Deliberately not "openinvar-at-": that is the prefix the temporary
        // checkout uses, and a fixture sharing it would make the leak assertions
        // fire on the repository's own path.
        let root = std::env::temp_dir().join(format!(
            "openinvar-fixture-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&root).expect("create repo dir");

        git(&root, &["init", "--quiet"]);
        // Identity and signing are set locally so the fixture commits whatever
        // the machine's global git config says.
        git(&root, &["config", "user.email", "test@example.invalid"]);
        git(&root, &["config", "user.name", "OpenInvar Test"]);
        git(&root, &["config", "commit.gpgsign", "false"]);

        std::fs::write(root.join("alpha.ts"), "export function alpha() { return 1; }\n")
            .expect("write alpha");
        git(&root, &["add", "."]);
        git(&root, &["commit", "--quiet", "-m", "add alpha"]);
        let first = git(&root, &["rev-parse", "HEAD"]);

        std::fs::write(root.join("beta.ts"), "export function beta() { return 2; }\n")
            .expect("write beta");
        git(&root, &["add", "."]);
        git(&root, &["commit", "--quiet", "-m", "add beta"]);

        Self { root, first }
    }

    fn symbol_names(&self, json: &str) -> Vec<String> {
        let doc: serde_json::Value = serde_json::from_str(json)
            .unwrap_or_else(|e| panic!("stdout was not JSON: {e}\n{json}"));
        let mut names = Vec::new();
        for file in doc["files"].as_array().expect("files array") {
            for symbol in file["symbols"].as_array().expect("symbols array") {
                if let Some(name) = symbol["name"].as_str() {
                    names.push(name.to_string());
                }
            }
        }
        names.sort();
        names
    }

    fn worktrees(&self) -> String {
        git(&self.root, &["worktree", "list"])
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn at_analyses_the_revisions_tree_not_the_working_tree() {
    let repo = Repo::create("past");

    let (past, code) = openinvar(&repo.root, &["analyze", ".", "--at", &repo.first, "--json"]);
    assert_eq!(code, 0, "analyze --at failed:\n{past}");
    let past = repo.symbol_names(&past);

    let (now, _) = openinvar(&repo.root, &["analyze", ".", "--json"]);
    let now = repo.symbol_names(&now);

    assert!(
        past.iter().any(|n| n == "alpha"),
        "the first revision's symbol is missing: {past:?}"
    );
    assert!(
        !past.iter().any(|n| n == "beta"),
        "a symbol added after the requested revision appeared in it — \
         the working tree was analysed, not the revision: {past:?}"
    );
    assert!(
        now.iter().any(|n| n == "beta"),
        "the fixture is wrong: beta is not in the working tree either: {now:?}"
    );
}

#[test]
fn at_is_byte_identical_across_runs() {
    // The temporary checkout lands on a different path every run. If any part
    // of it reached the output, this is where it would show, and determinism is
    // the property every other claim in this project rests on.
    let repo = Repo::create("determinism");

    let (first, _) = openinvar(&repo.root, &["analyze", ".", "--at", &repo.first, "--json"]);
    let (second, _) = openinvar(&repo.root, &["analyze", ".", "--at", &repo.first, "--json"]);

    assert_eq!(
        first, second,
        "two analyses of one revision disagreed byte for byte"
    );
}

#[test]
fn at_reports_the_repository_not_the_checkout() {
    let repo = Repo::create("root");

    let (json, _) = openinvar(&repo.root, &["analyze", ".", "--at", &repo.first, "--json"]);
    let doc: serde_json::Value = serde_json::from_str(&json).expect("JSON");
    let root = doc["root"].as_str().expect("root is a string");

    let expected = std::fs::canonicalize(&repo.root).expect("canonicalize repo");
    assert_eq!(
        std::fs::canonicalize(root).expect("canonicalize reported root"),
        expected,
        "the report names something other than the repository"
    );
    assert!(
        !root.contains("openinvar-at-"),
        "the report names the temporary checkout instead of the repository: {root}"
    );
    // The reported root is the one absolute path in the document, and every
    // path inside `files` is relative to it — so naming the checkout would make
    // every file path in the report resolve against a directory that no longer
    // exists by the time anyone reads it.
    assert!(
        !json.contains("openinvar-at-"),
        "the temporary checkout path leaked into the report body"
    );
}

#[test]
fn at_leaves_the_working_graph_alone() {
    // `query`, `check` and `audit` read the working graph when nobody names a
    // revision. If `--at` overwrote it, every later command would answer about
    // a past revision without saying so.
    let repo = Repo::create("working-graph");

    openinvar(&repo.root, &["analyze", "."]);
    let (before, _) = openinvar(&repo.root, &["query", "usages", "beta"]);

    openinvar(&repo.root, &["analyze", ".", "--at", &repo.first]);
    let (after, _) = openinvar(&repo.root, &["query", "usages", "beta"]);

    assert_eq!(
        before, after,
        "--at changed what the working graph answers"
    );
}

#[test]
fn at_does_not_disturb_uncommitted_work() {
    // Checking the revision out in place would be the obvious implementation
    // and would destroy exactly this.
    let repo = Repo::create("uncommitted");
    let scratch = repo.root.join("uncommitted.ts");
    std::fs::write(&scratch, "export function gamma() { return 3; }\n").expect("write");
    let head_before = git(&repo.root, &["rev-parse", "HEAD"]);

    openinvar(&repo.root, &["analyze", ".", "--at", &repo.first]);

    assert!(scratch.is_file(), "an uncommitted file was removed");
    assert_eq!(
        std::fs::read_to_string(&scratch).expect("read"),
        "export function gamma() { return 3; }\n",
        "an uncommitted file was modified"
    );
    assert_eq!(
        git(&repo.root, &["rev-parse", "HEAD"]),
        head_before,
        "--at moved HEAD; it checked the revision out in place"
    );
    assert!(
        !git(&repo.root, &["status", "--porcelain"]).is_empty(),
        "the uncommitted change is no longer pending"
    );
}

#[test]
fn at_records_a_revision_diff_can_read() {
    // The point of the flag: build a graph for a revision nobody checked out,
    // then compare it. Without this, a historical comparison needs a checkout
    // per revision.
    let repo = Repo::create("diff");

    openinvar(&repo.root, &["analyze", ".", "--at", &repo.first]);
    openinvar(&repo.root, &["analyze", ".", "--snapshot", "worktree"]);

    let (report, code) = openinvar(
        &repo.root,
        &["diff", ".", "--base", &repo.first, "--head", "worktree"],
    );
    assert_eq!(code, 0, "diff failed:\n{report}");
    assert!(
        report.contains("Symbols added"),
        "diff did not report against the recorded revision:\n{report}"
    );
}

#[test]
fn at_refuses_the_working_tree() {
    let repo = Repo::create("refuse");

    let (out, code) = openinvar(&repo.root, &["analyze", ".", "--at", "worktree"]);
    assert_ne!(code, 0, "--at worktree was accepted:\n{out}");
}

#[test]
fn at_refuses_an_unknown_revision() {
    let repo = Repo::create("unknown");

    let (out, code) = openinvar(&repo.root, &["analyze", ".", "--at", "no-such-revision"]);
    assert_ne!(code, 0, "an unknown revision was accepted:\n{out}");
}

#[test]
fn at_leaves_no_worktree_behind() {
    let repo = Repo::create("cleanup");

    openinvar(&repo.root, &["analyze", ".", "--at", &repo.first]);
    // A failing run has to clean up too: the error paths are where a leak is
    // most likely, and a leaked registration makes the *next* run fail.
    openinvar(&repo.root, &["analyze", ".", "--at", "no-such-revision"]);

    let listed = repo.worktrees();
    assert_eq!(
        listed.lines().count(),
        1,
        "a temporary worktree was left registered:\n{listed}"
    );
}
