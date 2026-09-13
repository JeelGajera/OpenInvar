//! What changed between two graphs.
//!
//! The cases that matter are the ones where a naive comparison is loudly
//! wrong: a rename read as a delete plus an add tells every consumer above
//! that a symbol was destroyed, which is the largest possible description of
//! the smallest possible change.

use openinvar_core::delta::{self, Continuation};
use openinvar_core::graph::InvarGraph;
use openinvar_core::incremental::replace_file_ir;
use openinvar_core::ir::{
    FileIR, Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind,
};

fn symbol(file: &str, name: &str, kind: SymbolKind, line: u32, signature: Option<&str>) -> Symbol {
    Symbol {
        id: format!("{file}::{name}::{kind:?}").to_lowercase(),
        name: name.to_string(),
        kind,
        language: Language::TypeScript,
        file: file.to_string(),
        line_start: line,
        line_end: line,
        signature: signature.map(str::to_string),
    }
}

fn edge(from: &str, to: &str, file: &str, resolution: Resolution) -> Relationship {
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

fn graph_of(files: Vec<(&str, Vec<Symbol>, Vec<Relationship>)>) -> InvarGraph {
    let mut graph = InvarGraph::new();
    for (file, symbols, relationships) in files {
        replace_file_ir(
            &mut graph,
            &FileIR {
                file: file.to_string(),
                language: Language::TypeScript,
                symbols,
                relationships,
                diagnostics: vec![],
                re_exports: vec![],
            },
        );
    }
    graph
}

#[test]
fn an_unchanged_graph_produces_an_empty_delta() {
    let build = || {
        graph_of(vec![(
            "a.ts",
            vec![symbol("a.ts", "Alpha", SymbolKind::Class, 1, Some("class Alpha"))],
            vec![],
        )])
    };
    let d = delta::compute(&build(), &build());
    assert!(d.is_empty(), "identical graphs differed: {d:?}");
}

#[test]
fn an_added_symbol_is_reported_as_added() {
    let before = graph_of(vec![("a.ts", vec![], vec![])]);
    let after = graph_of(vec![(
        "a.ts",
        vec![symbol("a.ts", "Alpha", SymbolKind::Class, 1, None)],
        vec![],
    )]);

    let d = delta::compute(&before, &after);
    assert_eq!(d.added_symbols.len(), 1, "{d:?}");
    assert_eq!(d.added_symbols[0].name, "Alpha");
    assert!(d.removed_symbols.is_empty());
    assert!(d.continuities.is_empty());
}

#[test]
fn a_removed_symbol_is_reported_as_removed() {
    let before = graph_of(vec![(
        "a.ts",
        vec![symbol("a.ts", "Alpha", SymbolKind::Class, 1, None)],
        vec![],
    )]);
    let after = graph_of(vec![("a.ts", vec![], vec![])]);

    let d = delta::compute(&before, &after);
    assert_eq!(d.removed_symbols.len(), 1, "{d:?}");
    assert_eq!(d.removed_symbols[0].name, "Alpha");
    assert!(d.added_symbols.is_empty());
}

#[test]
fn a_rename_is_one_continuity_not_a_delete_and_an_add() {
    // The case the whole module exists for.
    let before = graph_of(vec![(
        "a.ts",
        vec![symbol(
            "a.ts",
            "Alpha",
            SymbolKind::Class,
            4,
            Some("class Alpha { id: string }"),
        )],
        vec![],
    )]);
    let after = graph_of(vec![(
        "a.ts",
        vec![symbol(
            "a.ts",
            "Beta",
            SymbolKind::Class,
            4,
            Some("class Beta { id: string }"),
        )],
        vec![],
    )]);

    let d = delta::compute(&before, &after);
    assert_eq!(d.continuities.len(), 1, "{d:?}");
    assert_eq!(d.continuities[0].how, Continuation::Renamed);
    assert_eq!(d.continuities[0].before.name, "Alpha");
    assert_eq!(d.continuities[0].after.name, "Beta");
    assert!(
        d.added_symbols.is_empty() && d.removed_symbols.is_empty(),
        "a rename also produced a delete/add pair: {d:?}"
    );
}

#[test]
fn a_move_between_files_is_a_continuity_and_says_so() {
    let before = graph_of(vec![(
        "a.ts",
        vec![symbol("a.ts", "Alpha", SymbolKind::Class, 1, Some("class Alpha"))],
        vec![],
    )]);
    let after = graph_of(vec![(
        "b.ts",
        vec![symbol("b.ts", "Alpha", SymbolKind::Class, 1, Some("class Alpha"))],
        vec![],
    )]);

    let d = delta::compute(&before, &after);
    assert_eq!(d.continuities.len(), 1, "{d:?}");
    assert_eq!(d.continuities[0].how, Continuation::Moved);
    assert_eq!(d.continuities[0].before.file, "a.ts");
    assert_eq!(d.continuities[0].after.file, "b.ts");
}

#[test]
fn a_signature_change_keeps_the_symbol_and_reports_the_change() {
    let before = graph_of(vec![(
        "a.ts",
        vec![symbol("a.ts", "Alpha", SymbolKind::Function, 1, Some("f(a: number)"))],
        vec![],
    )]);
    let after = graph_of(vec![(
        "a.ts",
        vec![symbol("a.ts", "Alpha", SymbolKind::Function, 1, Some("f(a: string)"))],
        vec![],
    )]);

    let d = delta::compute(&before, &after);
    assert_eq!(d.signature_changes.len(), 1, "{d:?}");
    assert_eq!(d.signature_changes[0].after.signature.as_deref(), Some("f(a: string)"));
    assert!(d.added_symbols.is_empty() && d.removed_symbols.is_empty());
}

#[test]
fn a_kind_change_is_not_treated_as_a_continuity() {
    // A function replaced by a class of the same name is a real change. Pairing
    // them would hide it behind a rename that did not happen.
    let before = graph_of(vec![(
        "a.ts",
        vec![symbol("a.ts", "Alpha", SymbolKind::Function, 1, None)],
        vec![],
    )]);
    let after = graph_of(vec![(
        "a.ts",
        vec![symbol("a.ts", "Alpha", SymbolKind::Class, 1, None)],
        vec![],
    )]);

    let d = delta::compute(&before, &after);
    assert!(d.continuities.is_empty(), "kinds were paired across: {d:?}");
    assert_eq!(d.added_symbols.len(), 1);
    assert_eq!(d.removed_symbols.len(), 1);
}

#[test]
fn an_unsigned_rename_at_a_different_line_stays_a_delete_and_an_add() {
    // No signature and no shared position is not evidence. Pairing on kind
    // alone would match every removed class in a file with every added one.
    let before = graph_of(vec![(
        "a.ts",
        vec![symbol("a.ts", "Alpha", SymbolKind::Class, 3, None)],
        vec![],
    )]);
    let after = graph_of(vec![(
        "a.ts",
        vec![symbol("a.ts", "Beta", SymbolKind::Class, 90, None)],
        vec![],
    )]);

    let d = delta::compute(&before, &after);
    assert!(
        d.continuities.is_empty(),
        "a pairing was invented without evidence: {d:?}"
    );
    assert_eq!(d.added_symbols.len(), 1);
    assert_eq!(d.removed_symbols.len(), 1);
}

#[test]
fn each_symbol_is_paired_at_most_once() {
    // Two removals that both look like one addition must not both claim it.
    let before = graph_of(vec![(
        "a.ts",
        vec![
            symbol("a.ts", "Alpha", SymbolKind::Class, 1, Some("shared")),
            symbol("a.ts", "Gamma", SymbolKind::Class, 1, Some("shared")),
        ],
        vec![],
    )]);
    let after = graph_of(vec![(
        "a.ts",
        vec![symbol("a.ts", "Beta", SymbolKind::Class, 1, Some("shared"))],
        vec![],
    )]);

    let d = delta::compute(&before, &after);
    assert_eq!(d.continuities.len(), 1, "one addition was claimed twice: {d:?}");
    assert_eq!(
        d.removed_symbols.len(),
        1,
        "the unpaired removal was dropped instead of reported: {d:?}"
    );
}

#[test]
fn edges_are_reported_added_and_removed() {
    let before = graph_of(vec![
        ("a.ts", vec![symbol("a.ts", "Alpha", SymbolKind::Class, 1, None)], vec![]),
        (
            "b.ts",
            vec![symbol("b.ts", "Beta", SymbolKind::Class, 1, None)],
            vec![edge("b.ts::beta::class", "a.ts::alpha::class", "b.ts", Resolution::Resolved)],
        ),
    ]);
    let after = graph_of(vec![
        ("a.ts", vec![symbol("a.ts", "Alpha", SymbolKind::Class, 1, None)], vec![]),
        ("b.ts", vec![symbol("b.ts", "Beta", SymbolKind::Class, 1, None)], vec![]),
    ]);

    let d = delta::compute(&before, &after);
    assert_eq!(d.removed_edges.len(), 1, "{d:?}");
    assert!(d.added_edges.is_empty());
    assert_eq!(d.removed_edges[0].resolution, Resolution::Resolved);
}

#[test]
fn a_delta_of_only_structural_edges_is_not_a_resolved_change() {
    // What lets a gate fail open on Tier 2: the graph changed, and nothing a
    // gate may act on did.
    let before = graph_of(vec![
        ("a.java", vec![symbol("a.java", "Alpha", SymbolKind::Class, 1, None)], vec![]),
        (
            "b.java",
            vec![symbol("b.java", "Beta", SymbolKind::Class, 1, None)],
            vec![edge("b.java::beta::class", "a.java::alpha::class", "b.java", Resolution::Structural)],
        ),
    ]);
    let after = graph_of(vec![
        ("a.java", vec![symbol("a.java", "Alpha", SymbolKind::Class, 1, None)], vec![]),
        ("b.java", vec![symbol("b.java", "Beta", SymbolKind::Class, 1, None)], vec![]),
    ]);

    let d = delta::compute(&before, &after);
    assert!(!d.is_empty(), "the structural edge change was not seen at all");
    assert!(
        !delta::has_resolved_changes(&d),
        "a structural-only change was reported as actionable: {d:?}"
    );
}

#[test]
fn the_delta_is_deterministic_across_repeated_runs() {
    // The product's first guarantee, asserted on the type every later gate
    // reads. A `HashMap` anywhere in `compute` would fail this intermittently.
    let build_before = || {
        graph_of(vec![(
            "a.ts",
            vec![
                symbol("a.ts", "Alpha", SymbolKind::Class, 1, Some("a")),
                symbol("a.ts", "Gamma", SymbolKind::Class, 2, Some("g")),
            ],
            vec![],
        )])
    };
    let build_after = || {
        graph_of(vec![(
            "a.ts",
            vec![
                symbol("a.ts", "Beta", SymbolKind::Class, 1, Some("a")),
                symbol("a.ts", "Delta", SymbolKind::Class, 2, Some("g")),
            ],
            vec![],
        )])
    };

    let first = delta::compute(&build_before(), &build_after());
    for _ in 0..8 {
        assert_eq!(
            delta::compute(&build_before(), &build_after()),
            first,
            "two runs over identical graphs produced different deltas"
        );
    }
}

#[test]
fn a_replaced_function_on_the_same_line_is_not_a_rename() {
    // Found end to end rather than in a unit test: deleting `doomed` and
    // adding `freshlyAdded` in its place put both on the same line, and a
    // position-only heuristic paired them as a rename. Replacing one function
    // with another is exactly what that looks like, so position alone cannot
    // be evidence — the substituted signature is what separates them.
    let before = graph_of(vec![(
        "m.ts",
        vec![symbol(
            "m.ts",
            "doomed",
            SymbolKind::Function,
            5,
            Some("function doomed(): number { return 2; }"),
        )],
        vec![],
    )]);
    let after = graph_of(vec![(
        "m.ts",
        vec![symbol(
            "m.ts",
            "freshlyAdded",
            SymbolKind::Function,
            5,
            Some("function freshlyAdded(): number { return 3; }"),
        )],
        vec![],
    )]);

    let d = delta::compute(&before, &after);
    assert!(
        d.continuities.is_empty(),
        "two unrelated functions sharing a line were paired as a rename: {d:?}"
    );
    assert_eq!(d.removed_symbols.len(), 1, "{d:?}");
    assert_eq!(d.added_symbols.len(), 1, "{d:?}");
}

#[test]
fn a_rename_whose_body_also_changed_records_no_continuity() {
    // The cost of the rule above, stated rather than discovered: once the body
    // moves too, nothing distinguishes a rename from an unrelated addition.
    // Reporting a removal and an addition is the honest answer, and inventing
    // a rename is the worse error.
    let before = graph_of(vec![(
        "m.ts",
        vec![symbol(
            "m.ts",
            "alpha",
            SymbolKind::Function,
            1,
            Some("function alpha(): number { return 1; }"),
        )],
        vec![],
    )]);
    let after = graph_of(vec![(
        "m.ts",
        vec![symbol(
            "m.ts",
            "beta",
            SymbolKind::Function,
            1,
            Some("function beta(): number { return 99; }"),
        )],
        vec![],
    )]);

    let d = delta::compute(&before, &after);
    assert!(d.continuities.is_empty(), "{d:?}");
    assert_eq!(d.removed_symbols.len(), 1);
    assert_eq!(d.added_symbols.len(), 1);
}
