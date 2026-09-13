//! How the CLI describes an aliased reference.
//!
//! This is presentation, but it is the presentation of the one fact OpenInvar
//! exists to surface. Both branches of the alias line used to print "imports
//! as X" — for "renamed to X" and for "imported by X", which are opposite
//! meanings. A reader could not tell which was meant.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch_copy(fixture: &str, test_name: &str) -> PathBuf {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(fixture);
    let dest =
        std::env::temp_dir().join(format!("openinvar-labels-{test_name}-{}", std::process::id()));
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

fn query(root: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_openinvar"))
        .arg("query")
        .args(args)
        .arg("--path")
        .arg(root)
        .output()
        .expect("run openinvar query");
    assert!(
        output.status.success(),
        "query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn a_rename_is_described_as_a_rename() {
    // The fixture imports `UserPayload as ResponseModel`. That is the finding
    // a text search for `UserPayload` would miss, and it has to read as one.
    let root = scratch_copy("alias-import-bug", "rename");
    let out = query(&root, &["blast-radius", "UserPayload"]);

    assert!(
        out.contains("renamed to") && out.contains("ResponseModel"),
        "expected the rename to be named as such:\n{out}"
    );
    assert!(
        !out.contains("imports as"),
        "the ambiguous wording must be gone:\n{out}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_module_scope_referrer_is_not_described_as_an_alias() {
    // Most import edges come from a file's module scope, and the old fallback
    // branch rendered that as "imports as module" — which reads as a rename to
    // the name `module`. The location line already says where the reference
    // is, so there is nothing to add.
    let root = scratch_copy("adapter-rust", "module-scope");
    let out = query(
        &root,
        &[
            "blast-radius",
            "UserPayload",
            "--file",
            "alias_import_bug/src/models/user_payload.rs",
        ],
    );

    assert!(
        !out.contains("imports as module"),
        "a module-scope referrer must not be reported as an alias:\n{out}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn one_reference_at_one_location_is_reported_once() {
    // The row-inflation fix, end to end: no file:line pair should appear more
    // than once under a single relationship kind.
    let root = scratch_copy("adapter-ts", "inflation");
    let out = query(
        &root,
        &[
            "blast-radius",
            "UserRepository",
            "--file",
            "di_injection/src/user.repository.ts",
        ],
    );

    let mut rows: Vec<&str> = out
        .lines()
        .filter(|l| l.contains(".ts:"))
        .map(str::trim)
        .collect();
    let before = rows.len();
    rows.sort_unstable();
    rows.dedup();
    assert_eq!(
        before,
        rows.len(),
        "the same location was reported more than once:\n{out}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_alias_count_reports_aliases_not_symbols() {
    // `status` printed `alias_chains.len()`, which counts symbols that have
    // aliases. Labelled "Alias chains", that read as a count of renames.
    let root = scratch_copy("alias-import-bug", "counts");
    let output = Command::new(env!("CARGO_BIN_EXE_openinvar"))
        .arg("status")
        .arg(&root)
        .output()
        .expect("run openinvar status");
    let out = String::from_utf8_lossy(&output.stdout);

    assert!(
        out.contains("Aliases"),
        "the count must say what it counts:\n{out}"
    );
    assert!(
        out.contains("across"),
        "both numbers are true and different; report both:\n{out}"
    );

    let _ = std::fs::remove_dir_all(&root);
}
