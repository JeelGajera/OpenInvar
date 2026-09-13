//! `graph_diff` and `check_rules` as an agent meets them.
//!
//! These tools return prose into a context window, and a model acts on the
//! sentences rather than inspecting the structure behind them. So the tests
//! are about the sentences: what the tool says when it has nothing, when it
//! could not look, and when it looked and found nothing are three different
//! answers, and only one of them is a pass.
//!
//! The failure being guarded against is a tool that says "no violations"
//! because no rule was evaluated. A person reading a terminal notices the
//! empty section; a model reading the summary does not.

use std::path::{Path, PathBuf};

use openinvar_core::graph::InvarGraph;
use openinvar_mcp::tools::{check_rules, graph_diff};

fn fixture(name: &str) -> PathBuf {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/alias-import-bug");
    let dest = std::env::temp_dir().join(format!("openinvar-mcp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dest);
    copy_tree(&src, &dest);
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

fn write_rules(root: &Path, body: &str) {
    let dir = root.join(".openinvar");
    std::fs::create_dir_all(&dir).expect("create .openinvar");
    std::fs::write(dir.join("rules.toml"), body).expect("write rules");
}

const FIELD_RULE: &str = r#"
[[rule]]
name = "payload-is-stable"
kind = "no-field-removal"
symbol = "UserPayload"
"#;

// ── check_rules ──────────────────────────────────────────────

#[test]
fn a_repository_with_no_rules_file_is_told_that_nothing_was_enforced() {
    let root = fixture("norules");
    let graph = InvarGraph::new();

    let answer = check_rules::execute(
        &root,
        &graph,
        check_rules::CheckRulesParams {
            base: None,
            head: None,
        },
    )
    .expect("no rules file is not an error");

    assert!(answer.contains("No rules file"));
    assert!(
        answer.contains("not a pass"),
        "an agent told 'no violations' by a repository with no rules draws \
         exactly the wrong conclusion:\n{answer}"
    );
}

#[test]
fn a_delta_rule_without_a_delta_is_reported_as_skipped_and_not_as_clean() {
    let root = fixture("skipped");
    write_rules(&root, FIELD_RULE);
    let graph = InvarGraph::new();

    let answer = check_rules::execute(
        &root,
        &graph,
        check_rules::CheckRulesParams {
            base: None,
            head: None,
        },
    )
    .expect("evaluating without a delta is allowed");

    assert!(answer.contains("SKIPPED"), "{answer}");
    assert!(
        !answer.contains("none was violated"),
        "the only rule was never evaluated:\n{answer}"
    );
    assert!(
        answer.contains("incomplete check") || answer.contains("not a pass"),
        "the summary must not read as clean:\n{answer}"
    );
}

#[test]
fn a_malformed_rules_file_is_an_error_that_says_nothing_was_checked() {
    let root = fixture("malformed");
    write_rules(
        &root,
        r#"
[[rule]]
name = "typo"
kind = "forbid-dependancy"
from = "a/**"
to = "b/**"
"#,
    );
    let graph = InvarGraph::new();

    let error = check_rules::execute(
        &root,
        &graph,
        check_rules::CheckRulesParams {
            base: None,
            head: None,
        },
    )
    .expect_err("an unreadable rules file must not read as a pass");

    assert!(
        error.contains("no rule was checked"),
        "the error must say the check did not happen:\n{error}"
    );
}

#[test]
fn base_without_head_is_refused() {
    let root = fixture("halfrev");
    write_rules(&root, FIELD_RULE);
    let graph = InvarGraph::new();

    let error = check_rules::execute(
        &root,
        &graph,
        check_rules::CheckRulesParams {
            base: Some("HEAD".to_string()),
            head: None,
        },
    )
    .expect_err("half a comparison is not a comparison");

    assert!(error.contains("must be given together"));
}

// ── graph_diff ───────────────────────────────────────────────

#[test]
fn a_revision_that_was_never_recorded_is_an_error_naming_the_fix() {
    // Analysing here instead would be the tempting shortcut and the wrong
    // one: the answer would describe whatever was on disk at the moment of
    // the call rather than the revision the agent asked about.
    let root = fixture("norevision");

    let error = graph_diff::execute(
        &root,
        graph_diff::GraphDiffParams {
            base: Some("worktree".to_string()),
            head: Some("worktree".to_string()),
        },
    )
    .expect_err("there is no snapshot and no graph");

    assert!(
        error.contains("snapshot") || error.contains("graph store"),
        "the error must name what is missing:\n{error}"
    );
}

#[test]
fn an_unresolvable_revision_is_refused_rather_than_used_as_a_name() {
    // A typo silently becoming its own snapshot name would diff against an
    // empty graph, which reads as "everything was added".
    let root = fixture("badrevision");

    let error = graph_diff::execute(
        &root,
        graph_diff::GraphDiffParams {
            base: Some("definitely-not-a-revision-xyz".to_string()),
            head: Some("worktree".to_string()),
        },
    )
    .expect_err("an unknown revision is an error");

    assert!(error.contains("not a revision"), "{error}");
}
