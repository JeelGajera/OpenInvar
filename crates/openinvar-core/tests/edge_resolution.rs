//! Per-edge resolution, and the verdict that rests on it.
//!
//! The claim `blast-radius` makes when it finds nothing — "safe to modify" —
//! is about the whole repository. It holds only if the whole graph was
//! resolved well enough to make it. Structural analysis cannot see across
//! files, so a reference living in a structural region would never have
//! reached the graph, and an empty result there is partly an artefact of how
//! much was resolved rather than a fact about the code.
//!
//! These cover the distinction itself and, more importantly, the two ways it
//! could quietly stop protecting anyone: the verdict forgetting to check, and
//! a mixed graph reporting itself as fully resolved.

use openinvar_core::graph::InvarGraph;
use openinvar_core::ir::{
    Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind,
};
use openinvar_core::query::{is_gate_safe, structural_files};

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
        context: "test".to_string(),
        file: file.to_string(),
        line: 1,
        resolution,
    }
}

fn graph_with(edges: &[(&str, Resolution)]) -> InvarGraph {
    let mut graph = InvarGraph::new();
    graph.add_symbol(symbol("a::A::class", "A", "a.ts"));
    graph.add_symbol(symbol("b::B::class", "B", "b.ts"));
    for (file, resolution) in edges {
        graph.add_relationship(&rel("a::A::class", "b::B::class", file, *resolution));
    }
    graph
}

#[test]
fn only_resolved_is_gate_safe() {
    // The whole point of the distinction. If this ever became true for
    // Structural, every gate would start trusting intra-file-only data.
    assert!(Resolution::Resolved.is_gate_safe());
    assert!(!Resolution::Structural.is_gate_safe());
}

#[test]
fn the_weaker_resolution_is_the_default() {
    // A construction site that forgets the field, or a stored graph written
    // before it existed, must under-claim rather than over-claim. Flipping
    // this would make every such edge silently gate-safe.
    assert_eq!(Resolution::default(), Resolution::Structural);
    assert!(!Resolution::default().is_gate_safe());
}

#[test]
fn a_fully_resolved_graph_is_gate_safe() {
    let graph = graph_with(&[("a.ts", Resolution::Resolved)]);
    assert!(is_gate_safe(&graph));
    assert!(structural_files(&graph).is_empty());
}

#[test]
fn one_structural_edge_makes_the_whole_graph_unsafe_to_conclude_from() {
    // This is the case the feature exists for: a polyglot repository where
    // most of the graph is resolved and one region is not. Reporting the graph
    // as gate-safe because most of it is would be the exact false reassurance
    // OpenInvar is built to prevent.
    let graph = graph_with(&[
        ("resolved.ts", Resolution::Resolved),
        ("structural.java", Resolution::Structural),
    ]);

    assert!(!is_gate_safe(&graph));
    let files = structural_files(&graph);
    assert_eq!(files.len(), 1);
    assert!(
        files.contains("structural.java"),
        "the structural file must be named so a reader can see what was not resolved: {files:?}"
    );
    assert!(
        !files.contains("resolved.ts"),
        "a resolved file must not be reported as a blind spot: {files:?}"
    );
}

#[test]
fn an_empty_graph_is_not_gate_safe() {
    // Nothing was resolved, so an empty result from it is evidence of nothing.
    // Returning true here would make "no edges at all" read as "fully verified
    // and nothing depends on this" — the most dangerous possible inversion.
    let graph = graph_with(&[]);
    assert!(!is_gate_safe(&graph));
}

#[test]
fn structural_files_are_reported_once_each() {
    // Ordered and deduplicated, because the list reaches a user and OpenInvar's
    // first guarantee is that identical input produces identical output.
    let graph = graph_with(&[
        ("b.java", Resolution::Structural),
        ("a.java", Resolution::Structural),
        ("b.java", Resolution::Structural),
    ]);

    let files: Vec<String> = structural_files(&graph).into_iter().collect();
    assert_eq!(files, vec!["a.java".to_string(), "b.java".to_string()]);
}
