//! Where a repository's configuration lives, and what happens when it moves.
//!
//! The location is the whole point of this file existing. Rules were specified
//! to live at `.openinvar/rules.toml`, inside the directory every repository
//! gitignores because the graph database is there — so the file holding a
//! repository's versioned constraints could not be versioned, and `check` had
//! nothing to enforce in any repository anywhere. These tests pin the fix and
//! the one-release bridge that stops an upgrade dropping rules in silence.

use std::path::PathBuf;

use openinvar_core::rules::{self, Located};

fn temp_root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "openinvar-config-{name}-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp root");
    dir
}

const ONE_RULE: &str = r#"
[[rule]]
name = "core-does-not-depend-on-cli"
kind = "forbid-dependency"
from = "crates/core/**"
to   = "crates/cli/**"
"#;

fn write_root(root: &std::path::Path, body: &str) {
    std::fs::write(root.join("openinvar.toml"), body).expect("write root config");
}

fn write_legacy(root: &std::path::Path, body: &str) {
    let dir = root.join(".openinvar");
    std::fs::create_dir_all(&dir).expect("create legacy dir");
    std::fs::write(dir.join("rules.toml"), body).expect("write legacy config");
}

#[test]
fn a_repository_with_no_configuration_is_not_an_error() {
    // The ordinary case. A repository that has written no rules is not
    // misconfigured, and reporting it as such would make the tool unusable
    // everywhere it has not been adopted yet.
    let root = temp_root("absent");
    assert_eq!(rules::locate(&root), None);
}

#[test]
fn the_root_file_is_where_configuration_is_read_from() {
    let root = temp_root("root");
    write_root(&root, ONE_RULE);

    match rules::locate(&root) {
        Some(Located::Root(path)) => assert_eq!(path, root.join("openinvar.toml")),
        other => panic!("expected the root file, got {other:?}"),
    }
}

#[test]
fn a_pre_release_file_is_still_read_and_marked_deprecated() {
    // The bridge. Dropping these on upgrade would be a gate that stopped
    // enforcing without saying so — the failure this project is built against.
    let root = temp_root("legacy");
    write_legacy(&root, ONE_RULE);

    let found = rules::locate(&root).expect("the old location is still read");
    assert!(found.is_legacy(), "it must be reported as deprecated");
    assert_eq!(found.path(), root.join(".openinvar").join("rules.toml"));

    let parsed = rules::load(found.path()).expect("and still parses");
    assert_eq!(parsed.rules.len(), 1);
}

#[test]
fn the_root_file_wins_when_both_exist() {
    // Mid-migration. Reading both and merging would make the effective rule
    // set depend on two files nobody is looking at together.
    let root = temp_root("both");
    write_legacy(&root, ONE_RULE);
    write_root(
        &root,
        r#"
[[rule]]
name = "the-one-that-counts"
kind = "max-fan-in"
threshold = 40
"#,
    );

    let found = rules::locate(&root).expect("located");
    assert!(!found.is_legacy(), "the root file must win");

    let parsed = rules::load(found.path()).expect("parsed");
    assert_eq!(parsed.rules.len(), 1);
    assert_eq!(parsed.rules[0].name, "the-one-that-counts");
}

#[test]
fn suppressions_are_read_from_the_same_file() {
    let root = temp_root("suppress");
    write_root(
        &root,
        r#"
[[rule]]
name = "fan-in"
kind = "max-fan-in"
threshold = 40

[suppress]
"test-tampering-1a2b3c4d" = "the test was rewritten when the API changed"
"contract-erosion-99887766" = ""
"#,
    );

    let parsed = rules::load(&root.join("openinvar.toml")).expect("parsed");
    assert_eq!(parsed.rules.len(), 1);
    assert_eq!(parsed.suppress.len(), 2);
    assert_eq!(
        parsed.suppress.get("test-tampering-1a2b3c4d").map(String::as_str),
        Some("the test was rewritten when the API changed")
    );
}

#[test]
fn a_mistyped_section_is_rejected_rather_than_ignored() {
    // The reason `deny_unknown_fields` is on. `[supress]` with one 'p' parsed
    // happily before and silenced nothing, and a suppression that silently
    // does nothing is the same class of lie as one that silently does
    // something — which is what this file's own doc comment promises against.
    let root = temp_root("typo");
    write_root(
        &root,
        r#"
[[rule]]
name = "fan-in"
kind = "max-fan-in"
threshold = 40

[supress]
"test-tampering-1a2b3c4d" = "typo in the section name"
"#,
    );

    let err = rules::load(&root.join("openinvar.toml"))
        .expect_err("a section nobody reads must not parse");
    let text = err.to_string();
    assert!(text.contains("supress"), "the error must name it: {text}");
}

#[test]
fn a_mistyped_rule_array_does_not_read_as_an_empty_rule_set() {
    // `[[rules]]` plural produced zero rules and a clean run. A gate that
    // passes because its configuration was misspelled looks exactly like a
    // gate that passed on the merits.
    let root = temp_root("plural");
    write_root(
        &root,
        r#"
[[rules]]
name = "fan-in"
kind = "max-fan-in"
threshold = 40
"#,
    );

    assert!(
        rules::load(&root.join("openinvar.toml")).is_err(),
        "a misspelled rule array must not report zero rules and a pass"
    );
}
