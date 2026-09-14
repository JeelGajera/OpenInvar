//! `--min-confidence` / `min_resolution`: what a gate is allowed to see.
//!
//! The property that matters is not "rows below the floor are absent from the
//! output" — that much a filter over the result set would give you. It is that
//! nothing is *reached through* an edge below the floor, because a symbol two
//! hops away found by way of a guess is itself a guess.

use openinvar_core::graph::InvarGraph;
use openinvar_core::incremental::replace_file_ir;
use openinvar_core::ir::{
    FileIR, Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind,
};
use openinvar_core::query::{blast_radius, symbol_usages, RelationshipKindMask};

fn symbol(id: &str, name: &str, file: &str) -> Symbol {
    Symbol {
        id: id.to_string(),
        name: name.to_string(),
        kind: SymbolKind::Class,
        language: Language::TypeScript,
        file: file.to_string(),
        line_start: 1,
        line_end: 1,
        signature: None,
    }
}

fn rel(from: &str, to: &str, file: &str, resolution: Resolution) -> Relationship {
    Relationship {
        from: from.to_string(),
        to: to.to_string(),
        kind: RelationshipKind::Imports,
        alias: None,
        properties_accessed: vec![],
        context: "import".to_string(),
        file: file.to_string(),
        line: 1,
        resolution,
    }
}

fn file_ir(file: &str, symbols: Vec<Symbol>, relationships: Vec<Relationship>) -> FileIR {
    FileIR {
        file: file.to_string(),
        language: Language::TypeScript,
        symbols,
        relationships,
        diagnostics: vec![],
        re_exports: vec![],
        assertions: Default::default(),
        assertions_counted: false,
    }
}

/// `c -> b` is structural, `b -> a` is resolved.
///
/// Walking inbound from `a`: hop 1 reaches `b` on a resolved edge, hop 2
/// reaches `c` on a structural one.
fn chain() -> InvarGraph {
    let mut graph = InvarGraph::new();
    let a = symbol("a.ts::A::class", "A", "a.ts");
    let b = symbol("b.ts::B::class", "B", "b.ts");
    let c = symbol("c.ts::C::class", "C", "c.ts");

    replace_file_ir(&mut graph, &file_ir("a.ts", vec![a], vec![]));
    replace_file_ir(
        &mut graph,
        &file_ir(
            "b.ts",
            vec![b],
            vec![rel(
                "b.ts::B::class",
                "a.ts::A::class",
                "b.ts",
                Resolution::Resolved,
            )],
        ),
    );
    replace_file_ir(
        &mut graph,
        &file_ir(
            "c.ts",
            vec![c],
            vec![rel(
                "c.ts::C::class",
                "b.ts::B::class",
                "c.ts",
                Resolution::Structural,
            )],
        ),
    );
    graph
}

#[test]
fn the_permissive_floor_returns_every_edge() {
    let graph = chain();
    let edges = blast_radius(
        &graph,
        "A",
        None,
        Some(3),
        RelationshipKindMask::all(),
        Resolution::Structural,
    )
    .expect("query must succeed");

    assert_eq!(edges.len(), 2, "expected both hops, got {edges:?}");
}

#[test]
fn the_gate_floor_drops_the_structural_edge() {
    let graph = chain();
    let edges = blast_radius(
        &graph,
        "A",
        None,
        Some(3),
        RelationshipKindMask::all(),
        Resolution::Resolved,
    )
    .expect("query must succeed");

    assert_eq!(edges.len(), 1, "expected only the resolved hop, got {edges:?}");
    assert!(
        edges.iter().all(|e| e.resolution == Resolution::Resolved),
        "a structural edge survived the gate floor: {edges:?}"
    );
}

#[test]
fn nothing_is_reached_by_way_of_an_edge_below_the_floor() {
    // The property a post-hoc filter would get wrong. `C` is only reachable
    // through the structural `c -> b` edge, so at the gate floor `C` must not
    // appear at all — not merely have its own edge hidden.
    let graph = chain();
    let edges = blast_radius(
        &graph,
        "A",
        None,
        Some(3),
        RelationshipKindMask::all(),
        Resolution::Resolved,
    )
    .expect("query must succeed");

    assert!(
        edges.iter().all(|e| !e.from.contains("C") && !e.to.contains("C")),
        "C was reached through an untrusted edge: {edges:?}"
    );
}

#[test]
fn a_fully_structural_graph_answers_nothing_at_the_gate_floor() {
    // What a Tier 2 language looks like to a gate: the graph has content, and
    // the honest answer to "what may I act on" is none of it.
    let mut graph = InvarGraph::new();
    let a = symbol("a.java::A::class", "A", "a.java");
    let b = symbol("b.java::B::class", "B", "b.java");
    replace_file_ir(&mut graph, &file_ir("a.java", vec![a], vec![]));
    replace_file_ir(
        &mut graph,
        &file_ir(
            "b.java",
            vec![b],
            vec![rel(
                "b.java::B::class",
                "a.java::A::class",
                "b.java",
                Resolution::Structural,
            )],
        ),
    );

    let permissive = symbol_usages(
        &graph,
        "A",
        None,
        true,
        RelationshipKindMask::all(),
        Resolution::Structural,
    )
    .expect("query must succeed");
    assert_eq!(permissive.len(), 1, "{permissive:?}");

    let gated = symbol_usages(
        &graph,
        "A",
        None,
        true,
        RelationshipKindMask::all(),
        Resolution::Resolved,
    )
    .expect("query must succeed");
    assert!(
        gated.is_empty(),
        "a gate was handed structural rows to act on: {gated:?}"
    );
}

#[test]
fn the_ladder_orders_resolved_above_structural() {
    assert!(Resolution::Resolved.meets(Resolution::Structural));
    assert!(Resolution::Resolved.meets(Resolution::Resolved));
    assert!(Resolution::Structural.meets(Resolution::Structural));
    assert!(!Resolution::Structural.meets(Resolution::Resolved));
}
