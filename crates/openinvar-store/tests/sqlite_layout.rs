//! Properties of the SQLite layout itself, as distinct from the snapshot
//! encoding `sqlite_store.rs` covers and the revision contract `revisions.rs`
//! covers.
//!
//! These exist because the port from RocksDB changed what is guaranteed
//! underneath the same public API. RocksDB's prefix iterator returned keys in
//! order; SQLite returns rows in whatever order it likes unless a query says
//! otherwise. Everything here is about that difference, and about not
//! destroying a store written by the release before this one.

use std::path::PathBuf;

use openinvar_core::graph::InvarGraph;
use openinvar_core::incremental::replace_file_ir;
use openinvar_core::ir::{FileIR, Language, Symbol, SymbolKind};
use openinvar_store::sqlite::is_legacy_rocksdb_store;
use openinvar_store::{GraphSnapshot, GraphStore, StoreError};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "openinvar-layout-{name}-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

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
        unbound_references: 0,
        unbound_outside_repository: 0,
        references_counted: false,
    };
    replace_file_ir(&mut graph, &file_ir);
    graph
}

fn snapshot_of(name: &str) -> GraphSnapshot {
    GraphSnapshot::from_graph(&graph_with(name)).expect("snapshot")
}

#[test]
fn revision_order_is_stable_across_reopens_and_rewrites() {
    // The determinism trap this port introduced. RocksDB's prefix iterator was
    // ordered, so `list_revisions` was *accidentally* stable; SQLite guarantees
    // no row order without an ORDER BY. A missing one would not fail loudly —
    // it would make `openinvar diff` pick a different base on a re-run, which
    // is the failure mode this project has regressed into twice.
    //
    // Rewriting a revision in the middle is what would expose it: SQLite is
    // free to return an updated row at a different position, so an unordered
    // query would move `second` around while the sequence says exactly where
    // it belongs.
    let dir = temp_dir("order-stable");
    let expected = {
        let store = GraphStore::open(&dir).expect("open");
        for name in ["alpha", "beta", "gamma", "delta"] {
            store.save_revision(name, &snapshot_of("X")).expect("save");
        }
        store.save_revision("beta", &snapshot_of("Y")).expect("rewrite");
        store
            .list_revisions()
            .expect("list")
            .into_iter()
            .map(|e| e.revision)
            .collect::<Vec<_>>()
    };

    // The rewrite makes `beta` the most recent, ahead of everything present
    // when it was written.
    assert_eq!(expected, vec!["beta", "delta", "gamma", "alpha"]);

    for _ in 0..8 {
        let store = GraphStore::open(&dir).expect("reopen");
        let got: Vec<String> = store
            .list_revisions()
            .expect("list")
            .into_iter()
            .map(|e| e.revision)
            .collect();
        assert_eq!(got, expected, "revision order changed between reads");
    }
}

#[test]
fn a_rocksdb_store_directory_is_reported_rather_than_written_over() {
    // A user upgrading has a `.openinvar/db` directory holding a database this
    // build cannot read. Creating a SQLite database inside it, or removing it,
    // are both worse than saying so: the first leaves them wondering where
    // their revisions went, the second deletes data on their behalf.
    let dir = temp_dir("legacy");
    let old_store = dir.join("db");
    std::fs::create_dir_all(&old_store).expect("create old store");
    std::fs::write(old_store.join("CURRENT"), b"MANIFEST-000005\n").expect("write CURRENT");
    std::fs::write(old_store.join("MANIFEST-000005"), b"\x00\x01").expect("write manifest");

    assert!(is_legacy_rocksdb_store(&old_store));

    match GraphStore::open(&old_store) {
        Err(StoreError::LegacyStore(path)) => assert_eq!(path, old_store),
        Err(other) => panic!("expected LegacyStore, got {other}"),
        Ok(_) => panic!("a RocksDB store directory was opened as if it were ours"),
    }

    // The message has to name the remedy, not just the problem: the graph is
    // derived data, so rebuilding is the whole fix.
    let message = StoreError::LegacyStore(old_store.clone()).to_string();
    assert!(message.contains("openinvar analyze"), "{message}");
    assert!(message.contains("store format changed"), "{message}");

    // Reported, not repaired: everything that was there is still there.
    assert!(old_store.join("CURRENT").is_file());
    assert!(old_store.join("MANIFEST-000005").is_file());
    assert!(
        !old_store.join("graph.db").exists(),
        "a new database was written inside the old store"
    );
}

#[test]
fn an_ordinary_directory_is_not_mistaken_for_a_rocksdb_store() {
    // `is_dir()` alone would reject every directory, including the fresh one
    // every caller of `open` has always passed. The marker file is what
    // separates the two cases.
    let dir = temp_dir("ordinary");
    assert!(!is_legacy_rocksdb_store(&dir));

    let store = GraphStore::open(&dir).expect("a plain directory is a valid target");
    store.save_graph(&graph_with("Working")).expect("save");
    assert!(
        dir.join("graph.db").is_file(),
        "opening a directory should put the database inside it"
    );
    assert_eq!(store.load_graph().expect("load").symbols.len(), 1);
}

#[test]
fn a_missing_parent_directory_is_created_rather_than_an_error() {
    // `analyze` writes to `.openinvar/graph.db` in a repository that has never
    // been analysed, so the directory does not exist yet.
    let dir = temp_dir("nested");
    let db = dir.join("deeper").join(".openinvar").join("graph.db");

    let store = GraphStore::open(&db).expect("open");
    store.save_graph(&graph_with("Working")).expect("save");
    assert!(db.is_file());
}

#[test]
fn a_second_connection_reads_what_the_first_wrote() {
    // `openinvar watch` holds the store open while a hook invocation opens it
    // again. Under the default rollback journal a reader and a writer exclude
    // each other; WAL is what makes this work, so it is asserted rather than
    // assumed from a PRAGMA having been issued.
    let dir = temp_dir("concurrent");
    let writer = GraphStore::open(&dir).expect("open writer");
    writer.save_graph(&graph_with("Working")).expect("save");
    writer.save_revision("head", &snapshot_of("Recorded")).expect("save revision");

    let reader = GraphStore::open(&dir).expect("open reader while the first is live");
    assert_eq!(reader.load_graph().expect("load").symbols.len(), 1);
    assert_eq!(reader.load_revision("head").expect("load").symbols[0].name, "Recorded");

    // And a write through the second connection is visible to the first.
    reader.save_revision("base", &snapshot_of("Older")).expect("save through reader");
    assert_eq!(writer.list_revisions().expect("list").len(), 2);
}

#[test]
fn a_revisions_table_from_another_layout_is_dropped_rather_than_misread() {
    // A stale index pointing at snapshots this build cannot parse is silent
    // corruption in a command whose whole purpose is to be trusted. Revisions
    // are a cache of something reproducible from git, so discarding them is
    // the cheap side of that trade — but the working graph is not part of the
    // bargain and must survive.
    let dir = temp_dir("schema");
    let db = dir.join("graph.db");
    {
        let store = GraphStore::open(&db).expect("open");
        store.save_graph(&graph_with("Working")).expect("save graph");
        store.save_revision("head", &snapshot_of("Recorded")).expect("save revision");
    }

    // Stamp a layout this build does not know about.
    let conn = rusqlite::Connection::open(&db).expect("raw open");
    conn.pragma_update(None, "user_version", 99i32).expect("stamp");
    drop(conn);

    let store = GraphStore::open(&db).expect("reopen");
    assert!(
        store.list_revisions().expect("list").is_empty(),
        "revisions written under another layout were kept"
    );
    assert!(
        store.load_revision("head").is_err(),
        "a dropped revision was still readable"
    );
    assert_eq!(
        store.load_graph().expect("load graph").symbols.len(),
        1,
        "the working graph was discarded along with the revisions"
    );
}
