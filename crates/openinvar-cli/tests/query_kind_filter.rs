//! The `--kind` filter as a user meets it.
//!
//! The interesting cases here are not the happy path but the two ways a
//! filter can mislead: a name the tool does not know, and a name it knows but
//! no edge in this graph carries. Both would otherwise produce an empty result
//! that reads as "nothing depends on this" — the one answer OpenInvar must never
//! give without warrant.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch_copy(test_name: &str) -> PathBuf {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/alias-import-bug");
    let dest =
        std::env::temp_dir().join(format!("openinvar-kind-{test_name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dest);
    copy_tree(&src, &dest);

    let status = Command::new(env!("CARGO_BIN_EXE_openinvar"))
        .arg("analyze")
        .arg(&dest)
        .arg("--json")
        .output()
        .expect("analyze the fixture");
    assert!(status.status.success(), "fixture analysis must succeed");
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

fn query(root: &Path, args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_openinvar"))
        .arg("query")
        .args(args)
        .arg("--path")
        .arg(root)
        .output()
        .expect("run openinvar query");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        success: output.status.success(),
    }
}

#[test]
fn an_unknown_kind_is_rejected_rather_than_ignored() {
    // Ignoring it would answer a narrower question than the one asked and
    // present the result as the answer to theirs.
    let root = scratch_copy("unknown");
    let run = query(&root, &["blast-radius", "UserPayload", "--kind", "imprts"]);

    assert!(!run.success, "an unknown kind must fail the command");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_kind_absent_from_this_graph_says_so() {
    // A silent empty result here would read as "nothing instantiates this".
    //
    // The claim is deliberately about *this graph* rather than about what
    // OpenInvar implements. The fixture contains no `new` expression, so the
    // answer is the same whether or not instantiation edges are supported —
    // and it stays correct as languages gain call edges at different times,
    // which a hand-maintained list of unimplemented kinds could not do.
    let root = scratch_copy("absent-kind");
    let run = query(
        &root,
        &["blast-radius", "UserPayload", "--kind", "instantiates"],
    );

    assert!(run.success, "a known kind is a valid query: {}", run.stdout);
    assert!(
        run.stdout.contains("no instantiates edge exists in this graph"),
        "expected a warning naming the absent kind, got:\n{}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("safe to modify"),
        "a filtered empty result must never be reported as safety:\n{}",
        run.stdout
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_filtered_empty_result_is_not_called_safe() {
    // The unfiltered wording — "safe to modify" — is a claim about the whole
    // graph. Under a filter the tool searched a slice of it, so the claim
    // would be unearned even when the filter names a kind that does exist.
    // Nothing re-exports UserPayload in this fixture, so the result is empty
    // for an ordinary reason rather than an unimplemented one.
    let root = scratch_copy("filtered-empty");
    let run = query(
        &root,
        &["blast-radius", "UserPayload", "--kind", "re-exports"],
    );

    assert!(run.success, "{}", run.stdout);
    assert!(
        run.stdout.contains("selected kinds"),
        "an empty filtered result must say the search was narrowed:\n{}",
        run.stdout
    );
    assert!(!run.stdout.contains("safe to modify"), "{}", run.stdout);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn filtering_narrows_the_result() {
    // The fixture reaches UserPayload through both an import and property
    // access. Asking for one must return fewer edges than asking for nothing,
    // and each reported edge must carry the kind that was asked for.
    let root = scratch_copy("narrows");

    let all = query(&root, &["blast-radius", "UserPayload"]);
    let imports = query(&root, &["blast-radius", "UserPayload", "--kind", "imports"]);
    assert!(all.success && imports.success);

    let count = |s: &str| s.matches("[imports]").count() + s.matches("[accesses-property]").count();
    assert!(
        count(&imports.stdout) < count(&all.stdout),
        "filtering did not narrow the result:\nall:\n{}\nimports:\n{}",
        all.stdout,
        imports.stdout
    );
    assert!(
        !imports.stdout.contains("[accesses-property]"),
        "an imports-only query returned a property access:\n{}",
        imports.stdout
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_active_filter_is_reported_back() {
    // A short result under a filter must not be mistaken for a small blast
    // radius, so the query says what it searched.
    let root = scratch_copy("reported");
    let run = query(&root, &["blast-radius", "UserPayload", "--kind", "imports"]);

    assert!(run.success, "{}", run.stdout);
    assert!(
        run.stdout.contains("Kinds") && run.stdout.contains("imports"),
        "expected the active filter in the header, got:\n{}",
        run.stdout
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_unfiltered_query_reports_no_filter() {
    // The converse: without --kind there is nothing to report, and a "Kinds"
    // line would imply a narrowing that did not happen.
    let root = scratch_copy("unfiltered");
    let run = query(&root, &["blast-radius", "UserPayload"]);

    assert!(run.success, "{}", run.stdout);
    assert!(
        !run.stdout.contains("Kinds"),
        "an unfiltered query must not claim a filter:\n{}",
        run.stdout
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_kind_the_graph_does_contain_draws_no_warning() {
    // The inverse of the absent-kind test, and the one that would catch the
    // warning inverting. A graph that *has* call edges must not be told it has
    // none — that would be the false reassurance in reverse, discouraging a
    // filter that would have returned real dependents.
    let root = scratch_copy("present-kind");
    let run = query(&root, &["blast-radius", "UserPayload", "--kind", "imports"]);

    assert!(run.success, "a known kind is a valid query: {}", run.stdout);
    assert!(
        !run.stdout.contains("exists in this graph"),
        "warned about a kind the graph actually contains:\n{}",
        run.stdout
    );

    let _ = std::fs::remove_dir_all(&root);
}
