//! What the query layer claims about aliases, and how many rows it claims it in.
//!
//! Every case here is one where the tool previously said something untrue with
//! confidence: a same-name reference reported as an alias, a property nothing
//! touches reported as aliased-only, one reference reported as three
//! dependents, and twelve renames reported as one.

use openinvar_core::graph::InvarGraph;
use openinvar_core::ir::{Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind};
use openinvar_core::query::{
    blast_radius, is_aliased_only_property, is_renamed, QueryEdge, RelationshipKindMask,
};
use openinvar_core::resolver::{AliasEntry, AliasScope};

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

fn edge(to: &str, alias: Option<&str>, props: &[&str]) -> QueryEdge {
    QueryEdge {
        from: "src/consumer.ts::module::module".to_string(),
        to: to.to_string(),
        kind: RelationshipKind::Imports,
        file: "src/consumer.ts".to_string(),
        line: 1,
        alias: alias.map(str::to_string),
        properties_accessed: props.iter().map(|p| p.to_string()).collect(),
        context: String::new(),
        hop: 1,
        resolution: Resolution::Resolved,
    }
}

fn graph_with_target() -> InvarGraph {
    let mut graph = InvarGraph::new();
    graph.add_symbol(symbol(
        "src/models/user.ts::UserPayload::class",
        "UserPayload",
        "src/models/user.ts",
    ));
    graph
}

#[test]
fn a_same_name_alias_is_not_an_aliased_only_property() {
    // The bug: `is_aliased_only_property` tested `alias.is_some()` while
    // `partition_by_alias` had already been fixed to call `is_renamed`. One
    // call site was missed, so a plain `import { UserPayload }` — which
    // records the local name because it is the local name — labelled every
    // field it touched "(aliased import only)".
    let graph = graph_with_target();
    let edges = vec![edge(
        "src/models/user.ts::UserPayload::class",
        Some("UserPayload"),
        &["userId"],
    )];

    assert!(
        !is_renamed(&graph, &edges[0]),
        "an alias equal to the symbol's own name is not a rename"
    );
    assert!(
        !is_aliased_only_property(&graph, &edges, "userId"),
        "a direct access must not be reported as reachable only under an alias"
    );
}

#[test]
fn a_genuine_rename_still_marks_the_property() {
    // The case the label exists for: a text search for `UserPayload` finds
    // nothing here, so the property really is invisible without the graph.
    let graph = graph_with_target();
    let edges = vec![edge(
        "src/models/user.ts::UserPayload::class",
        Some("ResponseModel"),
        &["userId"],
    )];

    assert!(is_renamed(&graph, &edges[0]));
    assert!(is_aliased_only_property(&graph, &edges, "userId"));
}

#[test]
fn one_direct_access_clears_the_label_for_everyone() {
    // "Aliased only" is a claim about every reference to the property. One
    // reference under the type's own name makes it false.
    let graph = graph_with_target();
    let edges = vec![
        edge(
            "src/models/user.ts::UserPayload::class",
            Some("ResponseModel"),
            &["userId"],
        ),
        edge(
            "src/models/user.ts::UserPayload::class",
            Some("UserPayload"),
            &["userId"],
        ),
    ];

    assert!(!is_aliased_only_property(&graph, &edges, "userId"));
}

#[test]
fn a_property_nothing_touches_is_not_aliased_only() {
    // `all` over an empty iterator is true, so the old predicate labelled a
    // property with no matching edges "aliased only" — a claim about
    // references that do not exist.
    let graph = graph_with_target();
    let edges = vec![edge(
        "src/models/user.ts::UserPayload::class",
        Some("ResponseModel"),
        &["userId"],
    )];

    assert!(
        !is_aliased_only_property(&graph, &edges, "neverAccessed"),
        "a property with no references cannot be aliased-only"
    );
}

// ── row inflation ────────────────────────────────────────────

fn rel(from: &str, to: &str, kind: RelationshipKind, line: u32) -> Relationship {
    Relationship {
        from: from.to_string(),
        to: to.to_string(),
        kind,
        alias: Some("Principal".to_string()),
        properties_accessed: vec!["userId".to_string()],
        context: String::new(),
        file: "src/audit/audit.repository.ts".to_string(),
        line,
        resolution: Resolution::Resolved,
    }
}

#[test]
fn one_reference_attributed_at_two_levels_is_one_row() {
    // A single source location is attributed both at class level and at
    // method level, producing two edges that differ only in `from`. With
    // `from` in the dedupe key both survived, so 38 referencing files were
    // reported as 196 dependents and the aliased findings — the whole point
    // of the tool — sat below 160 rows of duplicates.
    let mut graph = InvarGraph::new();
    graph.add_symbol(symbol(
        "src/shared/user-payload.ts::UserPayload::class",
        "UserPayload",
        "src/shared/user-payload.ts",
    ));
    graph.add_symbol(symbol(
        "src/audit/audit.repository.ts::AuditRepository::class",
        "AuditRepository",
        "src/audit/audit.repository.ts",
    ));
    graph.add_symbol(symbol(
        "src/audit/audit.repository.ts::find::method",
        "find",
        "src/audit/audit.repository.ts",
    ));

    let target = "src/shared/user-payload.ts::UserPayload::class";
    graph.add_relationship(&rel(
        "src/audit/audit.repository.ts::AuditRepository::class",
        target,
        RelationshipKind::AccessesProperty,
        6,
    ));
    graph.add_relationship(&rel(
        "src/audit/audit.repository.ts::find::method",
        target,
        RelationshipKind::AccessesProperty,
        6,
    ));

    let edges = blast_radius(
        &graph,
        "UserPayload",
        None,
        Some(1),
        RelationshipKindMask::all(),
        Resolution::Structural,
    )
    .expect("blast radius succeeds");

    assert_eq!(
        edges.len(),
        1,
        "one reference at one location is one finding, got: {edges:#?}"
    );
}

#[test]
fn two_kinds_at_one_location_stay_two_rows() {
    // The converse, and the reason `kind` is still part of the key: a class
    // that both extends a base and reads a field on it states two different
    // facts, and collapsing them would hide one.
    let mut graph = InvarGraph::new();
    graph.add_symbol(symbol("b.ts::Base::class", "Base", "b.ts"));
    graph.add_symbol(symbol("d.ts::Derived::class", "Derived", "d.ts"));

    for kind in [
        RelationshipKind::Extends,
        RelationshipKind::AccessesProperty,
    ] {
        graph.add_relationship(&rel("d.ts::Derived::class", "b.ts::Base::class", kind, 7));
    }

    let edges = blast_radius(&graph, "Base", None, Some(1), RelationshipKindMask::all(), Resolution::Structural)
        .expect("blast radius succeeds");
    assert_eq!(edges.len(), 2, "different kinds are different facts");
}

#[test]
fn deduplication_keeps_the_shortest_path() {
    // Which row survives is decided by the ordering rather than by traversal
    // order, and the lowest hop is the one worth showing.
    let mut graph = InvarGraph::new();
    graph.add_symbol(symbol("a.ts::A::class", "A", "a.ts"));
    graph.add_symbol(symbol("b.ts::B::class", "B", "b.ts"));
    graph.add_symbol(symbol("c.ts::C::class", "C", "c.ts"));

    let mut direct = rel(
        "b.ts::B::class",
        "a.ts::A::class",
        RelationshipKind::Imports,
        3,
    );
    direct.file = "b.ts".to_string();
    graph.add_relationship(&direct);

    let mut indirect = rel(
        "c.ts::C::class",
        "b.ts::B::class",
        RelationshipKind::Imports,
        9,
    );
    indirect.file = "c.ts".to_string();
    graph.add_relationship(&indirect);

    let edges = blast_radius(&graph, "A", None, Some(3), RelationshipKindMask::all(), Resolution::Structural)
        .expect("blast radius succeeds");
    assert_eq!(edges.first().map(|e| e.hop), Some(1), "nearest first");
}

// ── alias counting ───────────────────────────────────────────

#[test]
fn twelve_renames_of_one_symbol_count_as_twelve() {
    // `alias_chains` is keyed by symbol, so its length counts symbols that
    // have aliases. Printed as "Alias chains", that reads as a count of
    // renames: a type imported under a different name by twelve files
    // reported `1`.
    let graph = graph_with_target();
    let target = "src/models/user.ts::UserPayload::class".to_string();

    let aliases: Vec<AliasEntry> = (0..12)
        .map(|i| AliasEntry {
            alias_name: format!("Principal{i}"),
            defined_in_file: format!("src/domain{i}/service.ts"),
            scope: AliasScope::ImportAlias,
        })
        .collect();
    graph.alias_chains.insert(target, aliases);

    assert_eq!(graph.alias_count(), 12, "twelve renames is twelve");
    assert_eq!(
        graph.aliased_symbol_count(),
        1,
        "of one symbol — both numbers are true, and they are different numbers"
    );
}
