//! Assertion counts surviving a snapshot round trip.
//!
//! These counts are the evidence behind an accusation that someone gutted a
//! test, and `audit --base HEAD` reads the base side out of a stored snapshot.
//! If the format dropped them, the base would report zero assertions, every
//! test would look emptied, and the detector would be at its loudest exactly
//! when it is most wrong. The format is versioned, so the reverse matters too:
//! a snapshot written before version 4 must read back as *not counted* rather
//! than as counted-zero.

use openinvar_core::graph::InvarGraph;
use openinvar_core::incremental::replace_file_ir;
use openinvar_core::ir::{FileIR, Language, Symbol, SymbolKind};
use openinvar_store::{GraphSnapshot, GraphStore};

fn temp_db(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "openinvar-assertions-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir.join("graph.db")
}

fn symbol(id: &str, line: u32) -> Symbol {
    Symbol {
        id: id.to_string(),
        name: id.to_string(),
        kind: SymbolKind::Function,
        language: Language::Rust,
        file: "a.rs".to_string(),
        line_start: line,
        line_end: line + 5,
        signature: None,
    }
}

fn graph_with_counts(counted: bool, counts: &[(&str, u32)]) -> InvarGraph {
    let mut graph = InvarGraph::new();
    replace_file_ir(
        &mut graph,
        &FileIR {
            file: "a.rs".to_string(),
            language: Language::Rust,
            symbols: vec![symbol("a.rs::one::function", 1), symbol("a.rs::two::function", 20)],
            relationships: vec![],
            diagnostics: vec![],
            re_exports: vec![],
            assertions: counts.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            assertions_counted: counted,
            unbound_references: 0,
            unbound_outside_repository: 0,
            references_counted: false,
        },
    );
    graph
}

/// Through the real store, which is the path `audit --base` actually takes.
fn round_trip(name: &str, graph: &InvarGraph) -> InvarGraph {
    let store = GraphStore::open(&temp_db(name)).expect("open store");
    store.save_graph(graph).expect("save");
    store.load_graph().expect("load")
}

#[test]
fn counts_survive_a_round_trip() {
    let graph = graph_with_counts(true, &[("a.rs::one::function", 3), ("a.rs::two::function", 1)]);
    let restored = round_trip("counts", &graph);

    assert_eq!(
        restored.assertions.get("a.rs::one::function").map(|v| *v),
        Some(3)
    );
    assert_eq!(
        restored.assertions.get("a.rs::two::function").map(|v| *v),
        Some(1)
    );
}

#[test]
fn the_counted_flag_survives_a_round_trip() {
    // Without the flag an empty map is ambiguous, and the ambiguity resolves
    // the wrong way: counted-zero supports a finding, uncounted does not.
    let counted = round_trip("flag-true", &graph_with_counts(true, &[]));
    assert_eq!(
        counted.assertions_counted.get("a.rs").map(|v| *v),
        Some(true),
        "a counted file came back as uncounted"
    );

    let uncounted = round_trip("flag-false", &graph_with_counts(false, &[]));
    assert_eq!(
        uncounted.assertions_counted.get("a.rs").map(|v| *v),
        Some(false),
        "an uncounted file came back as counted, which would license an accusation"
    );
}

#[test]
fn the_encoded_order_is_sorted_not_hash_order() {
    // Both maps come out of a DashMap, whose iteration order varies per
    // process, and the snapshot bytes are compared for equality by CI's
    // reproducibility check. The explicit sort is what makes that hold, and
    // this is what stops someone removing it.
    let graph = graph_with_counts(
        true,
        &[("a.rs::two::function", 2), ("a.rs::one::function", 7)],
    );
    let snapshot = GraphSnapshot::from_graph(&graph).expect("snapshot");

    assert!(
        snapshot.assertions.windows(2).all(|w| w[0].0 <= w[1].0),
        "assertion counts are not sorted by symbol id: {:?}",
        snapshot.assertions
    );
    assert!(
        snapshot
            .assertions_counted
            .windows(2)
            .all(|w| w[0].0 <= w[1].0),
        "counted flags are not sorted by file: {:?}",
        snapshot.assertions_counted
    );
}

#[test]
fn a_graph_that_counted_nothing_carries_no_counts() {
    // The shape an older snapshot decodes to: no counts, and the flag false.
    // A detector reading this must see "nothing was counted" rather than
    // "every test has zero assertions".
    let restored = round_trip("empty", &graph_with_counts(false, &[]));

    assert!(
        restored.assertions.is_empty(),
        "counts appeared for a graph that counted none"
    );
    assert_eq!(restored.assertions_counted.get("a.rs").map(|v| *v), Some(false));
}

// ── the unbound split ────────────────────────────────────────

/// A graph with unbound counts on two files, one partly classified.
fn graph_with_unbound(files: &[(&str, u32, u32)]) -> InvarGraph {
    let mut graph = InvarGraph::new();
    for (file, total, outside) in files {
        replace_file_ir(
            &mut graph,
            &FileIR {
                file: file.to_string(),
                language: Language::Rust,
                symbols: vec![],
                relationships: vec![],
                diagnostics: vec![],
                re_exports: vec![],
                assertions: Default::default(),
                assertions_counted: false,
                unbound_references: *total,
                unbound_outside_repository: *outside,
                references_counted: true,
            },
        );
    }
    graph
}

#[test]
fn the_unbound_split_survives_a_round_trip() {
    // `status` reads this back out of the store in a later process, so a format
    // that dropped the split would report every unbound reference as
    // unexplained — the loudest reading of the number, from a graph that
    // actually knew better.
    let graph = graph_with_unbound(&[("a.rs", 10, 4), ("b.rs", 3, 0)]);
    let restored = round_trip("unbound-split", &graph);

    assert_eq!(restored.unbound_references.get("a.rs").map(|v| *v), Some(10));
    assert_eq!(
        restored.unbound_outside_repository.get("a.rs").map(|v| *v),
        Some(4),
        "the classified share was lost in the store"
    );
    assert_eq!(
        restored.unbound_outside_repository.get("b.rs").map(|v| *v),
        Some(0),
        "a file that classified none came back missing rather than zero"
    );
}

#[test]
fn the_unbound_split_is_encoded_sorted() {
    // Same reason as the assertion counts above: it comes out of a DashMap, and
    // CI compares snapshot bytes for equality.
    let graph = graph_with_unbound(&[("z.rs", 4, 1), ("a.rs", 2, 2), ("m.rs", 6, 3)]);
    let snapshot = GraphSnapshot::from_graph(&graph).expect("snapshot");

    assert!(
        snapshot
            .unbound_outside_repository
            .windows(2)
            .all(|w| w[0].0 <= w[1].0),
        "the unbound split is not sorted by file: {:?}",
        snapshot.unbound_outside_repository
    );
}

#[test]
fn the_classified_share_never_exceeds_the_total_through_the_store() {
    // The two counts travel as separate sections of the same snapshot, so
    // nothing in the format ties them together.
    let graph = graph_with_unbound(&[("a.rs", 10, 4), ("b.rs", 3, 3), ("c.rs", 7, 0)]);
    let restored = round_trip("unbound-invariant", &graph);

    for entry in restored.unbound_references.iter() {
        let outside = restored
            .unbound_outside_repository
            .get(entry.key())
            .map(|v| *v)
            .unwrap_or(0);
        assert!(
            outside <= *entry.value(),
            "{}: classified {} of {} unbound references",
            entry.key(),
            outside,
            entry.value()
        );
    }
}
