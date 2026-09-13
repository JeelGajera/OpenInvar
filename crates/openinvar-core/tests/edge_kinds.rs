//! Filtering a traversal by relationship kind.
//!
//! `RelationshipMeta` has carried a `kind` since 0.2.0 and `traverse` threw it
//! away, so every consumer saw a property access and a trait implementation as
//! the same fact. These cover the plumbing and, more importantly, the two
//! places where getting it wrong would produce a confidently wrong answer:
//! traversal continuing through an excluded kind, and an unknown filter name
//! being ignored rather than rejected.

use openinvar_core::graph::InvarGraph;
use openinvar_core::ir::{Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind};
use openinvar_core::query::{
    self, blast_radius, dependencies, kinds_present, symbol_usages, RelationshipKindMask, ALL_KINDS,
};

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

fn rel(from: &str, to: &str, kind: RelationshipKind, file: &str, line: u32) -> Relationship {
    Relationship {
        from: from.to_string(),
        to: to.to_string(),
        kind,
        alias: None,
        properties_accessed: vec![],
        context: String::new(),
        file: file.to_string(),
        line,
        resolution: Resolution::Resolved,
    }
}

/// `Target` is reached by four consumers, each through a different kind.
fn graph_with_one_kind_each() -> InvarGraph {
    let mut graph = InvarGraph::new();
    let target = symbol("t.ts::Target::class", "Target", "t.ts");
    graph.add_symbol(target.clone());

    for (i, kind) in [
        RelationshipKind::Imports,
        RelationshipKind::Extends,
        RelationshipKind::UsesType,
        RelationshipKind::AccessesProperty,
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("c{i}.ts::C{i}::class");
        graph.add_symbol(symbol(&id, &format!("C{i}"), &format!("c{i}.ts")));
        graph.add_relationship(&rel(
            &id,
            &target.id,
            kind,
            &format!("c{i}.ts"),
            (i as u32) + 1,
        ));
    }
    graph
}

#[test]
fn every_edge_carries_its_kind() {
    let graph = graph_with_one_kind_each();
    let edges = blast_radius(&graph, "Target", None, Some(1), RelationshipKindMask::all(), Resolution::Structural)
        .expect("blast radius succeeds");

    assert_eq!(edges.len(), 4);
    let mut kinds: Vec<&str> = edges.iter().map(|e| query::kind_name(&e.kind)).collect();
    kinds.sort_unstable();
    assert_eq!(
        kinds,
        vec!["accesses-property", "extends", "imports", "uses-type"]
    );
}

#[test]
fn filtering_by_one_kind_returns_only_that_kind() {
    let graph = graph_with_one_kind_each();

    for kind in [
        RelationshipKind::Imports,
        RelationshipKind::Extends,
        RelationshipKind::UsesType,
        RelationshipKind::AccessesProperty,
    ] {
        let mask = RelationshipKindMask::from_kinds(std::slice::from_ref(&kind));
        let edges =
            blast_radius(&graph, "Target", None, Some(1), mask, Resolution::Structural).expect("blast radius succeeds");
        assert_eq!(
            edges.len(),
            1,
            "expected exactly one {} edge",
            query::kind_name(&kind)
        );
        assert_eq!(edges[0].kind, kind);
    }
}

#[test]
fn filtering_by_several_kinds_returns_their_union() {
    let graph = graph_with_one_kind_each();
    let mask = RelationshipKindMask::from_kinds(&[
        RelationshipKind::Imports,
        RelationshipKind::AccessesProperty,
    ]);
    let edges = blast_radius(&graph, "Target", None, Some(1), mask, Resolution::Structural).expect("blast radius succeeds");

    assert_eq!(edges.len(), 2);
    let mut kinds: Vec<&str> = edges.iter().map(|e| query::kind_name(&e.kind)).collect();
    kinds.sort_unstable();
    assert_eq!(kinds, vec!["accesses-property", "imports"]);
}

#[test]
fn an_excluded_kind_also_blocks_the_path_through_it() {
    // A imports B, B extends C. Asking "what imports C, transitively?" must
    // not return A: A reaches C only by way of an `extends` edge the caller
    // excluded. Filtering the collected result instead of the traversal would
    // return A at hop 2 and misreport how it depends on C.
    let mut graph = InvarGraph::new();
    let a = symbol("a.ts::A::class", "A", "a.ts");
    let b = symbol("b.ts::B::class", "B", "b.ts");
    let c = symbol("c.ts::C::class", "C", "c.ts");
    graph.add_symbol(a.clone());
    graph.add_symbol(b.clone());
    graph.add_symbol(c.clone());

    graph.add_relationship(&rel(&a.id, &b.id, RelationshipKind::Imports, "a.ts", 1));
    graph.add_relationship(&rel(&b.id, &c.id, RelationshipKind::Extends, "b.ts", 1));

    let unfiltered = blast_radius(&graph, "C", None, Some(3), RelationshipKindMask::all(), Resolution::Structural)
        .expect("unfiltered succeeds");
    assert_eq!(unfiltered.len(), 2, "both hops are reachable unfiltered");

    let imports_only = RelationshipKindMask::from_kinds(&[RelationshipKind::Imports]);
    let filtered =
        blast_radius(&graph, "C", None, Some(3), imports_only, Resolution::Structural).expect("filtered succeeds");
    assert!(
        filtered.is_empty(),
        "A reaches C only through an excluded `extends` edge, so nothing imports C"
    );
}

#[test]
fn dependencies_and_usages_filter_the_same_way() {
    let graph = graph_with_one_kind_each();

    let extends_only = RelationshipKindMask::from_kinds(&[RelationshipKind::Extends]);
    let usages = symbol_usages(&graph, "Target", None, true, extends_only, Resolution::Structural).expect("usages succeed");
    assert_eq!(usages.len(), 1);
    assert_eq!(usages[0].kind, RelationshipKind::Extends);

    // C0 imports Target, so from C0's side that edge is a dependency.
    let imports_only = RelationshipKindMask::from_kinds(&[RelationshipKind::Imports]);
    let deps = dependencies(&graph, "C0", None, Some(1), imports_only, Resolution::Structural).expect("deps succeed");
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].kind, RelationshipKind::Imports);

    let extends_deps = dependencies(&graph, "C0", None, Some(1), extends_only, Resolution::Structural).expect("deps");
    assert!(extends_deps.is_empty(), "C0 has no outgoing `extends` edge");
}

#[test]
fn two_edges_between_the_same_pair_survive_when_their_kinds_differ() {
    // A class that both extends a base and reads a field on it produces two
    // edges at one location. Deduplicating without the kind would collapse
    // them and hide precisely the distinction this change introduces.
    let mut graph = InvarGraph::new();
    let base = symbol("b.ts::Base::class", "Base", "b.ts");
    let derived = symbol("d.ts::Derived::class", "Derived", "d.ts");
    graph.add_symbol(base.clone());
    graph.add_symbol(derived.clone());

    graph.add_relationship(&rel(
        &derived.id,
        &base.id,
        RelationshipKind::Extends,
        "d.ts",
        7,
    ));
    graph.add_relationship(&rel(
        &derived.id,
        &base.id,
        RelationshipKind::AccessesProperty,
        "d.ts",
        7,
    ));

    let edges =
        blast_radius(&graph, "Base", None, Some(1), RelationshipKindMask::all(), Resolution::Structural).expect("ok");
    assert_eq!(edges.len(), 2, "same pair and line, different kinds");
}

#[test]
fn the_default_mask_admits_everything() {
    let mask = RelationshipKindMask::default();
    assert!(mask.is_all());
    for kind in ALL_KINDS {
        assert!(mask.contains(&kind), "default mask must admit every kind");
    }
    assert!(RelationshipKindMask::none().is_empty());
}

#[test]
fn kind_names_round_trip() {
    // The CLI and JSON both address kinds by these names, so a name that does
    // not parse back is a filter a user cannot express.
    for kind in ALL_KINDS {
        let name = query::kind_name(&kind);
        assert_eq!(
            query::parse_kind(name),
            Some(kind.clone()),
            "'{name}' does not round-trip"
        );
    }
    assert_eq!(query::parse_kind("no-such-kind"), None);
    assert_eq!(
        query::parse_kind("Imports"),
        None,
        "names are lowercase and hyphenated; accepting variants invites two spellings"
    );
}

#[test]
fn kinds_present_reports_only_what_the_graph_contains() {
    // The honesty warning is keyed off this. Asking the graph rather than a
    // hand-maintained constant is what keeps the warning true when a kind is
    // emitted for some languages but not others: the answer is about this
    // repository, not about which adapters exist.
    let mut graph = InvarGraph::new();
    graph.add_symbol(symbol("a::A::class", "A", "a.ts"));
    graph.add_symbol(symbol("b::B::class", "B", "b.ts"));
    graph.add_relationship(&rel(
        "a::A::class",
        "b::B::class",
        RelationshipKind::Imports,
        "a.ts",
        1,
    ));

    let present = kinds_present(&graph);
    assert!(present.contains(&RelationshipKind::Imports));
    assert!(
        !present.contains(&RelationshipKind::Calls),
        "a kind no edge carries must not be reported as present"
    );
    for kind in &present {
        assert!(ALL_KINDS.contains(kind));
    }
}

#[test]
fn kinds_present_is_empty_for_a_graph_with_no_edges() {
    // An empty graph must not claim to contain every kind, or the warning
    // inverts and stays silent exactly when it is most needed.
    let mut graph = InvarGraph::new();
    graph.add_symbol(symbol("a::A::class", "A", "a.ts"));
    assert!(kinds_present(&graph).is_empty());
}
