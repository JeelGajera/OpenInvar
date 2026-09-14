//! `no-cycles`.
//!
//! Finding cycles is not the work — petgraph ships Tarjan. The work is being
//! honest about the ones that cannot be seen.
//!
//! Running SCC over resolved edges and calling a clean result "no cycles" is
//! wrong: an edge that did not resolve could be the one that *closes* a cycle
//! the resolved edges leave open. So proving a scope acyclic means having
//! resolved every edge in it, and
//! `an_unresolved_edge_makes_acyclic_unproven` is the test that pins it.

use openinvar_core::graph::InvarGraph;
use openinvar_core::incremental::replace_file_ir;
use openinvar_core::ir::{
    FileIR, Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind,
};
use openinvar_core::rule_eval::{evaluate, RuleOutcome, Verdict};
use openinvar_core::rules;

fn symbol(file: &str) -> Symbol {
    Symbol {
        id: format!("{file}::sym"),
        name: "sym".to_string(),
        kind: SymbolKind::Function,
        language: Language::TypeScript,
        file: file.to_string(),
        line_start: 1,
        line_end: 2,
        signature: None,
    }
}

/// A graph from `from -> to` file pairs, every edge at the given resolution.
fn graph_of(pairs: &[(&str, &str)], resolution: Resolution) -> InvarGraph {
    let mut files: Vec<String> = Vec::new();
    for (from, to) in pairs {
        for f in [from, to] {
            if !files.iter().any(|seen| seen == f) {
                files.push(f.to_string());
            }
        }
    }

    let mut graph = InvarGraph::new();
    for file in &files {
        replace_file_ir(
            &mut graph,
            &FileIR {
                file: file.clone(),
                language: Language::TypeScript,
                symbols: vec![symbol(file)],
                relationships: vec![],
                diagnostics: vec![],
                re_exports: vec![],
            },
        );
    }
    for (from, to) in pairs {
        graph.add_relationship(&Relationship {
            from: symbol(from).id,
            to: symbol(to).id,
            kind: RelationshipKind::Imports,
            alias: None,
            properties_accessed: vec![],
            context: "test".to_string(),
            file: from.to_string(),
            line: 1,
            resolution,
        });
    }
    graph
}

const NO_CYCLES: &str = r#"
[[rule]]
name = "no-import-cycles"
kind = "no-cycles"
"#;

fn outcome(rules_text: &str, graph: &InvarGraph) -> RuleOutcome {
    let parsed = rules::parse(rules_text).expect("rules parse");
    evaluate(&parsed, graph, None).outcomes.remove(0)
}

#[test]
fn an_acyclic_graph_is_satisfied() {
    let graph = graph_of(&[("a.ts", "b.ts"), ("b.ts", "c.ts")], Resolution::Resolved);
    assert_eq!(outcome(NO_CYCLES, &graph).verdict, Verdict::Satisfied);
}

#[test]
fn a_two_file_cycle_is_reported_as_an_ordered_walk() {
    let graph = graph_of(&[("a.ts", "b.ts"), ("b.ts", "a.ts")], Resolution::Resolved);
    let found = outcome(NO_CYCLES, &graph);

    assert_eq!(found.verdict, Verdict::Violated);
    let detail = &found.violations[0].detail;
    // A set tells a reader which files are tangled; a walk tells them how.
    assert!(detail.contains("a.ts -> b.ts -> a.ts"), "{detail}");
}

#[test]
fn a_longer_cycle_renders_its_whole_path() {
    let graph = graph_of(
        &[("a.ts", "b.ts"), ("b.ts", "c.ts"), ("c.ts", "a.ts")],
        Resolution::Resolved,
    );
    let found = outcome(NO_CYCLES, &graph);

    assert_eq!(found.verdict, Verdict::Violated);
    assert!(
        found.violations[0].detail.contains("a.ts -> b.ts -> c.ts -> a.ts"),
        "{}",
        found.violations[0].detail
    );
}

#[test]
fn an_unresolved_edge_makes_acyclic_unproven() {
    // The test that justifies the second pass.
    //
    // These two edges form no cycle. But one of them did not resolve, so its
    // real target is unknown — and could be the node that closes a cycle.
    // Reporting Satisfied here would be claiming the scope was examined in
    // full when it was not.
    let graph = graph_of(&[("a.ts", "b.ts"), ("b.ts", "c.ts")], Resolution::Structural);
    let found = outcome(NO_CYCLES, &graph);

    assert_eq!(
        found.verdict,
        Verdict::Inconclusive,
        "an unresolved edge could close a cycle nobody can see"
    );
    assert!(found.uncertainty.expect("uncertainty").unresolved_edges > 0);
}

#[test]
fn a_cycle_found_on_resolved_edges_still_reports_even_beside_weak_ones() {
    // Violation beats uncertainty. Weak evidence elsewhere does not make a
    // cycle found on resolved evidence any less of a cycle.
    let mut graph = graph_of(&[("a.ts", "b.ts"), ("b.ts", "a.ts")], Resolution::Resolved);
    replace_file_ir(
        &mut graph,
        &FileIR {
            file: "weak.ts".to_string(),
            language: Language::TypeScript,
            symbols: vec![symbol("weak.ts")],
            relationships: vec![],
            diagnostics: vec![],
            re_exports: vec![],
        },
    );
    graph.add_relationship(&Relationship {
        from: symbol("weak.ts").id,
        to: symbol("a.ts").id,
        kind: RelationshipKind::Imports,
        alias: None,
        properties_accessed: vec![],
        context: "test".to_string(),
        file: "weak.ts".to_string(),
        line: 1,
        resolution: Resolution::Structural,
    });

    assert_eq!(outcome(NO_CYCLES, &graph).verdict, Verdict::Violated);
}

#[test]
fn a_file_depending_on_itself_is_not_a_cycle() {
    // A file referencing its own symbols is ordinary.
    let graph = graph_of(&[("a.ts", "a.ts")], Resolution::Resolved);
    assert_eq!(outcome(NO_CYCLES, &graph).verdict, Verdict::Satisfied);
}

#[test]
fn scope_narrows_which_cycles_count() {
    let rules_text = r#"
[[rule]]
name = "app-has-no-cycles"
kind = "no-cycles"
scope = "src/**"
"#;
    // The cycle is in vendor/, outside the scope.
    let graph = graph_of(
        &[("vendor/a.ts", "vendor/b.ts"), ("vendor/b.ts", "vendor/a.ts")],
        Resolution::Resolved,
    );
    assert_eq!(outcome(rules_text, &graph).verdict, Verdict::Satisfied);
}

#[test]
fn module_level_ignores_cycles_inside_one_directory() {
    // The Go case. Files inside one package reference each other freely, and
    // Go's own compiler already rejects circular *package* imports — so
    // file-level cycles there are noise rather than findings.
    let rules_text = r#"
[[rule]]
name = "no-package-cycles"
kind = "no-cycles"
level = "module"
"#;
    let graph = graph_of(
        &[("pkg/a.go", "pkg/b.go"), ("pkg/b.go", "pkg/a.go")],
        Resolution::Resolved,
    );
    assert_eq!(
        outcome(rules_text, &graph).verdict,
        Verdict::Satisfied,
        "two files in one package are not a package cycle"
    );

    // Across packages it is a real finding.
    let graph = graph_of(
        &[("one/a.go", "two/b.go"), ("two/b.go", "one/a.go")],
        Resolution::Resolved,
    );
    let found = outcome(rules_text, &graph);
    assert_eq!(found.verdict, Verdict::Violated);
    assert!(found.violations[0].detail.contains("one -> two -> one"), "{}", found.violations[0].detail);
}

#[test]
fn the_symbol_level_is_rejected_with_a_reason() {
    // Mutual recursion between functions is correct, ordinary code —
    // recursive descent parsers, visitors, state machines. A symbol level
    // would fire on every tokenizer in existence and be switched off in a
    // week, so it is refused rather than offered.
    let err = rules::parse(
        r#"
[[rule]]
name = "too-strict"
kind = "no-cycles"
level = "symbol"
"#,
    )
    .expect_err("symbol level must be refused");
    let text = err.to_string();
    assert!(text.contains("mutual recursion"), "{text}");
    assert!(text.contains("file"), "the error must name what to use instead: {text}");
}

#[test]
fn the_result_is_the_same_on_every_run() {
    // Tarjan's component order follows the graph's node numbering, which here
    // follows a BTreeSet rather than however the parallel scan finished. Two
    // runs over one repository must produce the same violation list.
    let graph = graph_of(
        &[
            ("a.ts", "b.ts"),
            ("b.ts", "a.ts"),
            ("x.ts", "y.ts"),
            ("y.ts", "x.ts"),
        ],
        Resolution::Resolved,
    );

    let first = outcome(NO_CYCLES, &graph);
    for _ in 0..6 {
        let again = outcome(NO_CYCLES, &graph);
        assert_eq!(
            first.violations, again.violations,
            "two evaluations of one graph disagreed"
        );
    }
    assert_eq!(first.violations.len(), 2, "both cycles are reported");
}
