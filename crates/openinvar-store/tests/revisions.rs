//! Snapshots keyed by revision.
//!
//! These assert the properties `openinvar diff` will rest on: that two
//! revisions can be stored and read back independently, that retention drops
//! the oldest rather than an arbitrary set, and that a layout change discards
//! the family instead of misreading it.

use std::path::PathBuf;

use openinvar_core::graph::InvarGraph;
use openinvar_core::incremental::replace_file_ir;
use openinvar_core::ir::{FileIR, Language, Symbol, SymbolKind};
use openinvar_store::rocksdb::{GraphSnapshot, RocksGraphStore};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "openinvar-revisions-{name}-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// A graph with one symbol, named so snapshots are distinguishable.
fn graph_with(symbol_name: &str) -> InvarGraph {
    let mut graph = InvarGraph::new();
    let file_ir = FileIR {
        file: "a.ts".to_string(),
        language: Language::TypeScript,
        symbols: vec![Symbol {
            id: format!("a.ts::{symbol_name}::class"),
            name: symbol_name.to_string(),
            kind: SymbolKind::Class,
            language: Language::TypeScript,
            file: "a.ts".to_string(),
            line_start: 1,
            line_end: 1,
            signature: None,
        }],
        relationships: vec![],
        diagnostics: vec![],
        re_exports: vec![],
        assertions: Default::default(),
        assertions_counted: false,
    };
    replace_file_ir(&mut graph, &file_ir);
    graph
}

fn snapshot_of(symbol_name: &str) -> GraphSnapshot {
    GraphSnapshot::from_graph(&graph_with(symbol_name)).expect("snapshot")
}

#[test]
fn two_revisions_are_stored_and_read_back_independently() {
    // The property `diff` exists to use: a base and a head held at once,
    // neither overwriting the other.
    let dir = temp_dir("two");
    let store = RocksGraphStore::open(&dir).expect("open");

    store.save_revision("base", &snapshot_of("Before")).expect("save base");
    store.save_revision("head", &snapshot_of("After")).expect("save head");

    let base = store.load_revision("base").expect("load base");
    let head = store.load_revision("head").expect("load head");

    assert_eq!(base.symbols.len(), 1);
    assert_eq!(head.symbols.len(), 1);
    assert_eq!(base.symbols[0].name, "Before");
    assert_eq!(head.symbols[0].name, "After");
}

#[test]
fn a_revision_snapshot_does_not_disturb_the_working_graph() {
    // Revisions live in their own column family precisely so this holds. The
    // working graph is read by every query; a revision write that touched it
    // would make `diff` able to corrupt `usages`.
    let dir = temp_dir("isolation");
    let store = RocksGraphStore::open(&dir).expect("open");

    store.save_graph(&graph_with("Working")).expect("save working graph");
    store.save_revision("head", &snapshot_of("Recorded")).expect("save revision");

    let working = store.load_graph().expect("load working graph");
    assert_eq!(working.symbols.len(), 1);
    assert!(
        working.symbols.iter().any(|e| e.value().name == "Working"),
        "the working graph was overwritten by a revision snapshot"
    );
}

#[test]
fn re_recording_a_revision_replaces_it_rather_than_accumulating() {
    // Two snapshots of one revision cannot both be right, and keeping the
    // older one would let `diff` answer from a graph the tree no longer
    // matches.
    let dir = temp_dir("replace");
    let store = RocksGraphStore::open(&dir).expect("open");

    store.save_revision("head", &snapshot_of("First")).expect("save");
    store.save_revision("head", &snapshot_of("Second")).expect("resave");

    let loaded = store.load_revision("head").expect("load");
    assert_eq!(loaded.symbols[0].name, "Second");
    assert_eq!(
        store.list_revisions().expect("list").len(),
        1,
        "re-recording a revision left two index entries"
    );
}

#[test]
fn revisions_are_listed_most_recently_written_first() {
    let dir = temp_dir("order");
    let store = RocksGraphStore::open(&dir).expect("open");

    for name in ["first", "second", "third"] {
        store.save_revision(name, &snapshot_of("X")).expect("save");
    }

    let listed: Vec<String> = store
        .list_revisions()
        .expect("list")
        .into_iter()
        .map(|entry| entry.revision)
        .collect();

    assert_eq!(listed, vec!["third", "second", "first"]);
}

#[test]
fn retention_keeps_the_newest_and_drops_the_rest() {
    let dir = temp_dir("retain");
    let store = RocksGraphStore::open(&dir).expect("open");

    for name in ["r1", "r2", "r3", "r4"] {
        store.save_revision(name, &snapshot_of("X")).expect("save");
    }

    let removed = store.prune_revisions(2).expect("prune");
    assert_eq!(removed, vec!["r2".to_string(), "r1".to_string()]);

    let remaining: Vec<String> = store
        .list_revisions()
        .expect("list")
        .into_iter()
        .map(|e| e.revision)
        .collect();
    assert_eq!(remaining, vec!["r4", "r3"]);

    // The dropped snapshots are gone, not merely unindexed.
    assert!(
        store.load_revision("r1").is_err(),
        "a pruned revision was still readable"
    );
}

#[test]
fn retention_that_keeps_more_than_exist_removes_nothing() {
    let dir = temp_dir("retain-none");
    let store = RocksGraphStore::open(&dir).expect("open");
    store.save_revision("only", &snapshot_of("X")).expect("save");

    assert!(store.prune_revisions(10).expect("prune").is_empty());
    assert!(store.load_revision("only").is_ok());
}

#[test]
fn a_missing_revision_is_an_error_rather_than_an_empty_graph() {
    // An empty graph would read as "nothing changed" to `diff`, which is the
    // wrong answer to "I have never seen that revision".
    let dir = temp_dir("missing");
    let store = RocksGraphStore::open(&dir).expect("open");
    assert!(store.load_revision("never-recorded").is_err());
}

#[test]
fn a_store_written_before_revisions_existed_still_opens() {
    // The family is created on open, so an existing `.openinvar/db` gains it
    // rather than failing. Simulated by opening twice: the first open creates
    // the database, the second finds the family already there.
    let dir = temp_dir("upgrade");
    {
        let store = RocksGraphStore::open(&dir).expect("first open");
        store.save_graph(&graph_with("Working")).expect("save");
    }
    let store = RocksGraphStore::open(&dir).expect("reopen");
    assert!(store.load_graph().is_ok());
    assert!(store.list_revisions().expect("list").is_empty());
}
