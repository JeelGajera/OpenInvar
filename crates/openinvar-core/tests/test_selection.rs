//! Selecting the tests that cover a change.
//!
//! This query is different from every other one here: its answer licenses an
//! *omission*. A developer or an agent reads it and does not run the rest of
//! the suite, so "these tests cover your change" is a claim that the ones left
//! out cannot fail.
//!
//! The tests below are therefore weighted towards the cases where the answer
//! must refuse to make that claim — a symbol nothing covers, a test the change
//! itself edited, evidence too weak to stand behind. A selection that is too
//! large costs minutes. One that is too small, presented as complete, is a
//! regression reaching main with a green tick beside it.

use std::collections::BTreeSet;

use openinvar_core::graph::InvarGraph;
use openinvar_core::incremental::replace_file_ir;
use openinvar_core::ir::{
    FileIR, Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind,
};
use openinvar_core::test_impact::{self, Gap};

fn symbol(file: &str, name: &str, kind: SymbolKind) -> Symbol {
    Symbol {
        id: format!("{file}::{name}::{kind:?}").to_lowercase(),
        name: name.to_string(),
        kind,
        language: Language::TypeScript,
        file: file.to_string(),
        line_start: 1,
        line_end: 5,
        signature: None,
    }
}

fn edge(from: &Symbol, to: &Symbol, kind: RelationshipKind, r: Resolution) -> Relationship {
    Relationship {
        from: from.id.clone(),
        to: to.id.clone(),
        kind,
        alias: None,
        properties_accessed: vec![],
        context: "t".to_string(),
        file: from.file.clone(),
        line: 1,
        resolution: r,
    }
}

fn graph_of(files: Vec<(&str, Vec<Symbol>, Vec<Relationship>)>) -> InvarGraph {
    let mut graph = InvarGraph::new();
    for (file, symbols, _) in &files {
        replace_file_ir(
            &mut graph,
            &FileIR {
                file: file.to_string(),
                language: Language::TypeScript,
                symbols: symbols.clone(),
                relationships: vec![],
                diagnostics: vec![],
                re_exports: vec![],
                assertions: Default::default(),
                assertions_counted: false,
                unbound_references: 0,
                unbound_outside_repository: 0,
                references_counted: false,
            },
        );
    }
    for (_, _, relationships) in &files {
        for r in relationships {
            graph.add_relationship(r);
        }
    }
    graph
}

/// `src/a.ts::target` exercised by `src/a.test.ts::covers_target`.
fn covered_graph(resolution: Resolution) -> (InvarGraph, Symbol, Symbol) {
    let target = symbol("src/a.ts", "target", SymbolKind::Function);
    let test = symbol("src/a.test.ts", "covers_target", SymbolKind::Function);
    let calls = edge(&test, &target, RelationshipKind::Calls, resolution);
    let tests = edge(&test, &target, RelationshipKind::Tests, resolution);
    let graph = graph_of(vec![
        ("src/a.ts", vec![target.clone()], vec![]),
        ("src/a.test.ts", vec![test.clone()], vec![calls, tests]),
    ]);
    (graph, target, test)
}

fn changed(ids: &[&Symbol]) -> BTreeSet<String> {
    ids.iter().map(|s| s.id.clone()).collect()
}

#[test]
fn a_test_that_exercises_the_changed_symbol_is_selected() {
    let (graph, target, test) = covered_graph(Resolution::Resolved);
    let selection = test_impact::select(&graph, &changed(&[&target]), 3, Resolution::Resolved);

    assert_eq!(selection.tests.len(), 1);
    assert_eq!(selection.tests[0].symbol, test.id);
    assert_eq!(selection.files().len(), 1);
    assert!(
        selection.is_trustworthy(),
        "every changed symbol is covered by a resolved edge: {:?}",
        selection.gaps
    );
}

#[test]
fn a_changed_symbol_nothing_covers_makes_the_selection_untrustworthy() {
    // The case the whole command exists to be honest about. Running only the
    // selected tests here would skip a symbol nothing exercises.
    let (mut graph, target, _) = covered_graph(Resolution::Resolved);
    let orphan = symbol("src/orphan.ts", "never_tested", SymbolKind::Function);
    replace_file_ir(
        &mut graph,
        &FileIR {
            file: "src/orphan.ts".to_string(),
            language: Language::TypeScript,
            symbols: vec![orphan.clone()],
            relationships: vec![],
            diagnostics: vec![],
            re_exports: vec![],
            assertions: Default::default(),
            assertions_counted: false,
            unbound_references: 0,
            unbound_outside_repository: 0,
            references_counted: false,
        },
    );

    let selection = test_impact::select(
        &graph,
        &changed(&[&target, &orphan]),
        3,
        Resolution::Resolved,
    );

    assert!(!selection.is_trustworthy());
    assert!(selection.uncovered.contains(&orphan.id));
    assert!(selection
        .gaps
        .iter()
        .any(|g| matches!(g, Gap::UncoveredSymbols(1))));
}

#[test]
fn a_test_the_change_modified_is_reported_apart_from_coverage() {
    // A test the diff edited is the thing whose behaviour changed, not
    // evidence that anything still works.
    let (graph, target, test) = covered_graph(Resolution::Resolved);
    let selection = test_impact::select(
        &graph,
        &changed(&[&target, &test]),
        3,
        Resolution::Resolved,
    );

    assert!(
        selection.tests.is_empty(),
        "a modified test must not be counted as coverage"
    );
    assert_eq!(selection.modified_tests.len(), 1);
    assert_eq!(selection.modified_tests[0].symbol, test.id);
}

#[test]
fn a_modified_test_does_not_mark_the_symbol_it_covers_as_covered() {
    // A diff that edits a function and its only test is the shape where a
    // reviewer most needs to be told something is missing. Letting the
    // modified test silence the gap would report full coverage instead.
    let (graph, target, test) = covered_graph(Resolution::Resolved);
    let selection = test_impact::select(
        &graph,
        &changed(&[&target, &test]),
        3,
        Resolution::Resolved,
    );

    assert!(selection.uncovered.contains(&target.id));
    assert!(!selection.is_trustworthy());
}

#[test]
fn a_test_selected_on_structural_evidence_is_flagged() {
    let (graph, target, _) = covered_graph(Resolution::Structural);
    let selection =
        test_impact::select(&graph, &changed(&[&target]), 3, Resolution::Structural);

    assert_eq!(selection.tests.len(), 1);
    assert!(!selection.tests[0].gate_safe);
    assert!(
        !selection.is_trustworthy(),
        "structural evidence cannot see across files"
    );
    assert!(selection
        .gaps
        .iter()
        .any(|g| matches!(g, Gap::StructuralEvidence(1))));
}

#[test]
fn coverage_reaches_through_an_intermediate_symbol() {
    // A test calling a wrapper that calls the changed function is covering it.
    let target = symbol("src/a.ts", "target", SymbolKind::Function);
    let wrapper = symbol("src/b.ts", "wrapper", SymbolKind::Function);
    let test = symbol("src/b.test.ts", "covers_wrapper", SymbolKind::Function);

    let w_to_t = edge(&wrapper, &target, RelationshipKind::Calls, Resolution::Resolved);
    let t_calls = edge(&test, &wrapper, RelationshipKind::Calls, Resolution::Resolved);
    let t_tests = edge(&test, &wrapper, RelationshipKind::Tests, Resolution::Resolved);

    let graph = graph_of(vec![
        ("src/a.ts", vec![target.clone()], vec![]),
        ("src/b.ts", vec![wrapper], vec![w_to_t]),
        ("src/b.test.ts", vec![test.clone()], vec![t_calls, t_tests]),
    ]);

    let deep = test_impact::select(&graph, &changed(&[&target]), 3, Resolution::Resolved);
    assert_eq!(deep.tests.len(), 1, "the wrapper's test covers the target");

    // At depth 0 only direct references count, so the same test does not.
    let shallow = test_impact::select(&graph, &changed(&[&target]), 0, Resolution::Resolved);
    assert!(shallow.tests.is_empty());
    assert!(!shallow.is_trustworthy());
}

#[test]
fn one_test_covering_two_changed_symbols_is_reported_once() {
    let (graph, target, _) = covered_graph(Resolution::Resolved);
    let selection = test_impact::select(&graph, &changed(&[&target]), 3, Resolution::Resolved);

    assert_eq!(selection.tests.len(), 1);
    assert_eq!(selection.files().len(), 1);
}

#[test]
fn an_empty_change_selects_nothing_and_claims_nothing() {
    let (graph, _, _) = covered_graph(Resolution::Resolved);
    let selection = test_impact::select(&graph, &BTreeSet::new(), 3, Resolution::Resolved);

    assert!(selection.tests.is_empty());
    assert!(selection.gaps.is_empty());
}

#[test]
fn selection_is_reproducible() {
    let (graph, target, _) = covered_graph(Resolution::Resolved);
    let ids = changed(&[&target]);
    let first = test_impact::select(&graph, &ids, 3, Resolution::Resolved);
    for _ in 0..6 {
        assert_eq!(
            test_impact::select(&graph, &ids, 3, Resolution::Resolved),
            first,
            "selection must not depend on hash iteration order"
        );
    }
}
