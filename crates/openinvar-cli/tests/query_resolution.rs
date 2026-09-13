//! The safety verdict, as a user meets it.
//!
//! `blast-radius` finding nothing prints "safe to modify". That is a claim
//! about the whole repository, and it holds only if the whole graph was
//! resolved well enough to make it. A structural region cannot see across
//! files, so a reference living in one would never have reached the graph and
//! the emptiness is partly an artefact of how much was resolved.
//!
//! The interesting case is not that the warning appears, but that it appears
//! only when warranted: a false warning on a fully resolved repository would
//! train people to ignore it, which costs more than never having shipped it.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(test_name: &str, sources: &[(&str, &str)]) -> PathBuf {
    let dest = std::env::temp_dir().join(format!(
        "openinvar-resolution-{test_name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dest);
    for (path, body) in sources {
        let file = dest.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).expect("create dirs");
        std::fs::write(&file, body).expect("write source");
    }

    let out = Command::new(env!("CARGO_BIN_EXE_openinvar"))
        .arg("analyze")
        .arg(&dest)
        .arg("--json")
        .output()
        .expect("analyze");
    assert!(out.status.success(), "fixture analysis must succeed");
    dest
}

fn query(root: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_openinvar"))
        .arg("query")
        .args(args)
        .arg("--path")
        .arg(root)
        .output()
        .expect("run openinvar query");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const SERVICES: &str = r#"
export class UserService { handle(): void {} }
export function unusedHelper(): void {}
"#;

const CONSUMER: &str = r#"
import { UserService } from "./services";
export function run(): void { const s = new UserService(); s.handle(); }
"#;

#[test]
fn a_fully_resolved_repository_still_gets_the_plain_verdict() {
    // The load-bearing negative case. If this warning fired everywhere it
    // would be noise, and noise gets ignored — including on the repositories
    // where it is the one thing standing between a user and a bad merge.
    let root = scratch(
        "resolved",
        &[("services.ts", SERVICES), ("consumer.ts", CONSUMER)],
    );
    let out = query(&root, &["blast-radius", "unusedHelper"]);

    assert!(
        out.contains("safe to modify"),
        "a fully resolved graph must still get the plain verdict:\n{out}"
    );
    assert!(
        !out.contains("structural regions"),
        "warned about structural regions in a graph that has none:\n{out}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_verdict_survives_a_reload_from_disk() {
    // Resolution is written to the snapshot and read back. When it was not,
    // every graph loaded from disk read back as structural: the safe
    // direction, so nothing failed loudly, but the plain verdict became
    // unreachable and the feature was silently useless. Querying goes through
    // the store, so this asserts the round trip end to end.
    let root = scratch(
        "reload",
        &[("services.ts", SERVICES), ("consumer.ts", CONSUMER)],
    );

    let first = query(&root, &["blast-radius", "unusedHelper"]);
    let second = query(&root, &["blast-radius", "unusedHelper"]);

    assert!(first.contains("safe to modify"), "{first}");
    assert_eq!(
        first.contains("structural regions"),
        second.contains("structural regions"),
        "the verdict changed between two reads of the same stored graph"
    );

    let _ = std::fs::remove_dir_all(&root);
}
