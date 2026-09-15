//! Turning a delta into judgements.
//!
//! The handoff's acceptance criterion for this layer is the first test: a
//! fixture commit removing a used symbol must produce a broken-edge finding
//! with the correct referrer list. The rest guard the ways a findings layer
//! goes wrong — reporting breakage that is not breakage, and reporting a guess
//! as a fact.

use openinvar_core::delta;
use openinvar_core::findings::{self, FindingKind};
use openinvar_core::graph::InvarGraph;
use openinvar_core::incremental::replace_file_ir;
use openinvar_core::ir::{
    FileIR, Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind,
};

fn symbol(file: &str, name: &str, kind: SymbolKind, sig: &str) -> Symbol {
    Symbol {
        id: format!("{file}::{name}::{kind:?}").to_lowercase(),
        name: name.to_string(),
        kind,
        language: Language::TypeScript,
        file: file.to_string(),
        line_start: 1,
        line_end: 1,
        signature: Some(sig.to_string()),
    }
}

fn edge(from: &str, to: &str, file: &str, resolution: Resolution) -> Relationship {
    Relationship {
        from: from.to_string(),
        to: to.to_string(),
        kind: RelationshipKind::Calls,
        alias: None,
        properties_accessed: vec![],
        context: "call".to_string(),
        file: file.to_string(),
        line: 7,
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
                assertions: Default::default(),
                assertions_counted: false,
                unbound_references: 0,
                references_counted: false,
            },
        );
    }
    graph
}

fn derive(before: &InvarGraph, after: &InvarGraph) -> findings::DiffFindings {
    let d = delta::compute(before, after);
    findings::derive(before, after, &d)
}

#[test]
fn removing_a_used_symbol_reports_a_broken_edge_naming_its_referrers() {
    // The acceptance criterion. `caller` survives and `target` does not, so
    // the reference `caller` still contains now points at nothing.
    let before = graph_of(vec![
        (
            "lib.ts",
            vec![symbol("lib.ts", "target", SymbolKind::Function, "function target() {}")],
            vec![],
        ),
        (
            "app.ts",
            vec![symbol("app.ts", "caller", SymbolKind::Function, "function caller() {}")],
            vec![edge(
                "app.ts::caller::function",
                "lib.ts::target::function",
                "app.ts",
                Resolution::Resolved,
            )],
        ),
    ]);
    let after = graph_of(vec![
        ("lib.ts", vec![], vec![]),
        (
            "app.ts",
            vec![symbol("app.ts", "caller", SymbolKind::Function, "function caller() {}")],
            vec![],
        ),
    ]);

    let result = derive(&before, &after);
    let broken: Vec<_> = result.of_kind(FindingKind::BrokenEdge).collect();

    assert_eq!(broken.len(), 1, "{:?}", result.findings);
    assert_eq!(broken[0].name, "target");
    assert_eq!(broken[0].referrers.len(), 1, "{:?}", broken[0]);
    assert_eq!(broken[0].referrers[0].symbol, "app.ts::caller::function");
    assert_eq!(broken[0].referrers[0].line, 7);
    assert_eq!(broken[0].resolution, Resolution::Resolved);
}

#[test]
fn removing_a_symbol_and_its_only_caller_together_is_not_breakage() {
    // Deleting a feature deletes both ends. Reporting that as a broken edge
    // would bury the ones that are, which is how a findings layer stops being
    // read at all.
    let before = graph_of(vec![
        (
            "lib.ts",
            vec![symbol("lib.ts", "target", SymbolKind::Function, "function target() {}")],
            vec![],
        ),
        (
            "app.ts",
            vec![symbol("app.ts", "caller", SymbolKind::Function, "function caller() {}")],
            vec![edge(
                "app.ts::caller::function",
                "lib.ts::target::function",
                "app.ts",
                Resolution::Resolved,
            )],
        ),
    ]);
    let after = graph_of(vec![("lib.ts", vec![], vec![]), ("app.ts", vec![], vec![])]);

    let result = derive(&before, &after);
    assert_eq!(
        result.of_kind(FindingKind::BrokenEdge).count(),
        0,
        "a clean removal was reported as breakage: {:?}",
        result.findings
    );
}

#[test]
fn a_broken_edge_from_a_structural_reference_is_advisory() {
    // The property that lets a gate fail open. The reference is real but was
    // matched by name inside one file, so the finding must not be actionable.
    let before = graph_of(vec![
        (
            "lib.java",
            vec![symbol("lib.java", "target", SymbolKind::Function, "void target() {}")],
            vec![],
        ),
        (
            "app.java",
            vec![symbol("app.java", "caller", SymbolKind::Function, "void caller() {}")],
            vec![edge(
                "app.java::caller::function",
                "lib.java::target::function",
                "app.java",
                Resolution::Structural,
            )],
        ),
    ]);
    let after = graph_of(vec![
        ("lib.java", vec![], vec![]),
        (
            "app.java",
            vec![symbol("app.java", "caller", SymbolKind::Function, "void caller() {}")],
            vec![],
        ),
    ]);

    let result = derive(&before, &after);
    let broken: Vec<_> = result.of_kind(FindingKind::BrokenEdge).collect();

    assert_eq!(broken.len(), 1, "{:?}", result.findings);
    assert_eq!(
        broken[0].resolution,
        Resolution::Structural,
        "a structural reference produced an actionable finding"
    );
    assert_eq!(
        result.gate_safe().filter(|f| f.kind == FindingKind::BrokenEdge).count(),
        0,
        "a gate was handed a structural broken edge to act on"
    );
}

#[test]
fn a_symbol_that_lost_its_last_referrer_is_orphaned() {
    let before = graph_of(vec![
        (
            "lib.ts",
            vec![symbol("lib.ts", "target", SymbolKind::Function, "function target() {}")],
            vec![],
        ),
        (
            "app.ts",
            vec![symbol("app.ts", "caller", SymbolKind::Function, "function caller() {}")],
            vec![edge(
                "app.ts::caller::function",
                "lib.ts::target::function",
                "app.ts",
                Resolution::Resolved,
            )],
        ),
    ]);
    // `target` stays; the call to it goes.
    let after = graph_of(vec![
        (
            "lib.ts",
            vec![symbol("lib.ts", "target", SymbolKind::Function, "function target() {}")],
            vec![],
        ),
        (
            "app.ts",
            vec![symbol("app.ts", "caller", SymbolKind::Function, "function caller() {}")],
            vec![],
        ),
    ]);

    let result = derive(&before, &after);
    let orphans: Vec<_> = result.of_kind(FindingKind::Orphaned).collect();
    assert_eq!(orphans.len(), 1, "{:?}", result.findings);
    assert_eq!(orphans[0].name, "target");
}

#[test]
fn a_symbol_nothing_ever_referred_to_is_not_newly_orphaned() {
    // Only a symbol that *lost* its referrers is a finding. One that never had
    // any is unremarkable, and reporting it would make every private helper a
    // finding on every diff.
    let before = graph_of(vec![(
        "lib.ts",
        vec![symbol("lib.ts", "lonely", SymbolKind::Function, "function lonely() {}")],
        vec![],
    )]);
    let after = graph_of(vec![(
        "lib.ts",
        vec![
            symbol("lib.ts", "lonely", SymbolKind::Function, "function lonely() {}"),
            symbol("lib.ts", "fresh", SymbolKind::Function, "function fresh() {}"),
        ],
        vec![],
    )]);

    let result = derive(&before, &after);
    assert_eq!(
        result.of_kind(FindingKind::Orphaned).count(),
        0,
        "{:?}",
        result.findings
    );
}

#[test]
fn removing_an_api_shaped_symbol_is_reported_even_with_no_referrers() {
    // Nothing in this repository used it, which says nothing about anything
    // outside it. A removed public method is a contract change whether or not
    // this graph can see a caller.
    let before = graph_of(vec![(
        "lib.ts",
        vec![symbol("lib.ts", "exported", SymbolKind::Method, "exported() {}")],
        vec![],
    )]);
    let after = graph_of(vec![("lib.ts", vec![], vec![])]);

    let result = derive(&before, &after);
    let api: Vec<_> = result.of_kind(FindingKind::ApiSurfaceRemoved).collect();
    assert_eq!(api.len(), 1, "{:?}", result.findings);
    assert_eq!(api[0].name, "exported");
}

#[test]
fn removing_a_local_variable_is_not_an_api_change() {
    let before = graph_of(vec![(
        "lib.ts",
        vec![symbol("lib.ts", "scratch", SymbolKind::Variable, "let scratch = 1")],
        vec![],
    )]);
    let after = graph_of(vec![("lib.ts", vec![], vec![])]);

    let result = derive(&before, &after);
    assert_eq!(
        result.of_kind(FindingKind::ApiSurfaceRemoved).count(),
        0,
        "{:?}",
        result.findings
    );
}

#[test]
fn a_signature_change_is_reported_with_who_depends_on_it() {
    let before = graph_of(vec![
        (
            "lib.ts",
            vec![symbol("lib.ts", "target", SymbolKind::Function, "function target(a: number) {}")],
            vec![],
        ),
        (
            "app.ts",
            vec![symbol("app.ts", "caller", SymbolKind::Function, "function caller() {}")],
            vec![edge(
                "app.ts::caller::function",
                "lib.ts::target::function",
                "app.ts",
                Resolution::Resolved,
            )],
        ),
    ]);
    let after = graph_of(vec![
        (
            "lib.ts",
            vec![symbol("lib.ts", "target", SymbolKind::Function, "function target(a: string) {}")],
            vec![],
        ),
        (
            "app.ts",
            vec![symbol("app.ts", "caller", SymbolKind::Function, "function caller() {}")],
            vec![edge(
                "app.ts::caller::function",
                "lib.ts::target::function",
                "app.ts",
                Resolution::Resolved,
            )],
        ),
    ]);

    let result = derive(&before, &after);
    let changed: Vec<_> = result.of_kind(FindingKind::SignatureChanged).collect();
    assert_eq!(changed.len(), 1, "{:?}", result.findings);
    assert_eq!(changed[0].referrers.len(), 1);
    assert!(
        result.blast_radius.contains("app.ts::caller::function"),
        "the dependent was not in the blast radius: {:?}",
        result.blast_radius
    );
}

#[test]
fn coverage_is_measured_over_the_changed_files_only() {
    // A change confined to one badly resolved file is not made trustworthy by
    // the rest of the repository resolving well, so the denominator is the
    // change rather than the repository.
    let before = graph_of(vec![
        (
            "clean.ts",
            vec![symbol("clean.ts", "a", SymbolKind::Function, "function a() {}")],
            vec![],
        ),
        (
            "murky.java",
            vec![symbol("murky.java", "b", SymbolKind::Function, "void b() {}")],
            vec![],
        ),
    ]);
    let after = graph_of(vec![
        (
            "clean.ts",
            vec![symbol("clean.ts", "a", SymbolKind::Function, "function a() {}")],
            vec![],
        ),
        (
            "murky.java",
            vec![
                symbol("murky.java", "b", SymbolKind::Function, "void b() {}"),
                symbol("murky.java", "c", SymbolKind::Function, "void c() {}"),
            ],
            vec![edge(
                "murky.java::c::function",
                "murky.java::b::function",
                "murky.java",
                Resolution::Structural,
            )],
        ),
    ]);

    let result = derive(&before, &after);
    assert_eq!(
        result.changed_file_coverage.percent(),
        Some(0.0),
        "changed-file coverage counted files the change never touched"
    );
}

#[test]
fn findings_are_deterministic() {
    let build_before = || {
        graph_of(vec![
            (
                "lib.ts",
                vec![
                    symbol("lib.ts", "one", SymbolKind::Function, "function one() {}"),
                    symbol("lib.ts", "two", SymbolKind::Function, "function two() {}"),
                ],
                vec![],
            ),
            (
                "app.ts",
                vec![symbol("app.ts", "caller", SymbolKind::Function, "function caller() {}")],
                vec![
                    edge("app.ts::caller::function", "lib.ts::one::function", "app.ts", Resolution::Resolved),
                    edge("app.ts::caller::function", "lib.ts::two::function", "app.ts", Resolution::Resolved),
                ],
            ),
        ])
    };
    let build_after = || {
        graph_of(vec![
            ("lib.ts", vec![], vec![]),
            (
                "app.ts",
                vec![symbol("app.ts", "caller", SymbolKind::Function, "function caller() {}")],
                vec![],
            ),
        ])
    };

    let first = derive(&build_before(), &build_after());
    for _ in 0..8 {
        assert_eq!(
            derive(&build_before(), &build_after()),
            first,
            "two runs over identical graphs produced different findings"
        );
    }
}

#[test]
fn a_stale_reference_in_an_edited_file_is_missed_and_that_is_recorded() {
    // The known false negative, pinned so it is a documented limit rather than
    // folklore, and so closing it fails this test loudly.
    //
    // `app.ts` is edited — `caller` gains a signature change — and it also
    // still refers to `target`, which is gone. Nothing distinguishes "the
    // author updated this caller" from "the author left a stale reference
    // here" in two resolved graphs: an unresolvable reference produces no edge
    // at all rather than a dangling one. The analyzer's own diagnostics say it
    // directly, but they are neither on the graph nor in a snapshot.
    let before = graph_of(vec![
        (
            "lib.ts",
            vec![symbol("lib.ts", "target", SymbolKind::Function, "function target() {}")],
            vec![],
        ),
        (
            "app.ts",
            vec![symbol("app.ts", "caller", SymbolKind::Function, "function caller() { target() }")],
            vec![edge(
                "app.ts::caller::function",
                "lib.ts::target::function",
                "app.ts",
                Resolution::Resolved,
            )],
        ),
    ]);
    let after = graph_of(vec![
        ("lib.ts", vec![], vec![]),
        (
            "app.ts",
            vec![symbol(
                "app.ts",
                "caller",
                SymbolKind::Function,
                "function caller() { target(); more() }",
            )],
            vec![],
        ),
    ]);

    let result = derive(&before, &after);
    assert_eq!(
        result.of_kind(FindingKind::BrokenEdge).count(),
        0,
        "this limit has been closed — update the doc comment on `derive` and \
         this test, which exists to make the gap visible"
    );
    // The removal is still reported through the other finding, so the change
    // is not invisible.
    assert_eq!(
        result.of_kind(FindingKind::ApiSurfaceRemoved).count(),
        1,
        "{:?}",
        result.findings
    );
}
