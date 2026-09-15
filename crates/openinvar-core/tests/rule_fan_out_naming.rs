//! `max-fan-out` and `naming-convention`.
//!
//! These two sit at opposite ends of the uncertainty question.
//!
//! `max-fan-out` mirrors fan-in and inherits its care: a breach is counted
//! from resolved edges only, and a symbol within the threshold whose weak
//! edges could carry it over is uncertainty rather than a pass.
//!
//! `naming-convention` is the only kind in the vocabulary that can never be
//! inconclusive. Names come from the parse rather than from resolution, so
//! even a file whose imports resolve to nothing reports the names it declares.

use openinvar_core::graph::InvarGraph;
use openinvar_core::incremental::replace_file_ir;
use openinvar_core::ir::{
    FileIR, Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind,
};
use openinvar_core::rule_eval::{evaluate, Verdict};
use openinvar_core::rules;

fn symbol(file: &str, name: &str, kind: SymbolKind) -> Symbol {
    Symbol {
        id: format!("{file}::{name}"),
        name: name.to_string(),
        kind,
        language: Language::TypeScript,
        file: file.to_string(),
        line_start: 1,
        line_end: 2,
        signature: None,
    }
}

fn edge(from: &Symbol, to: &Symbol, resolution: Resolution) -> Relationship {
    Relationship {
        from: from.id.clone(),
        to: to.id.clone(),
        kind: RelationshipKind::Imports,
        alias: None,
        properties_accessed: vec![],
        context: "test".to_string(),
        file: from.file.clone(),
        line: 1,
        resolution,
    }
}

fn build(files: &[(&str, Vec<Symbol>)], edges: &[Relationship]) -> InvarGraph {
    let mut graph = InvarGraph::new();
    for (file, symbols) in files {
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
                references_counted: false,
            },
        );
    }
    for relationship in edges {
        graph.add_relationship(relationship);
    }
    graph
}

/// One hub reaching `count` distinct targets, all at the given resolution.
fn hub_reaching(count: usize, resolution: Resolution) -> InvarGraph {
    let hub = symbol("src/hub.ts", "hub", SymbolKind::Function);
    let targets: Vec<Symbol> = (0..count)
        .map(|i| symbol(&format!("src/t{i}.ts"), &format!("t{i}"), SymbolKind::Function))
        .collect();

    let mut files: Vec<(&str, Vec<Symbol>)> = vec![("src/hub.ts", vec![hub.clone()])];
    let owned: Vec<String> = (0..count).map(|i| format!("src/t{i}.ts")).collect();
    for (i, name) in owned.iter().enumerate() {
        files.push((name.as_str(), vec![targets[i].clone()]));
    }
    let edges: Vec<Relationship> = targets.iter().map(|t| edge(&hub, t, resolution)).collect();
    build(&files, &edges)
}

fn outcome_for(rules_text: &str, graph: &InvarGraph) -> openinvar_core::rule_eval::RuleOutcome {
    let parsed = rules::parse(rules_text).expect("rules parse");
    evaluate(&parsed, graph, None).outcomes.remove(0)
}

const FAN_OUT_3: &str = r#"
[[rule]]
name = "no-god-module"
kind = "max-fan-out"
threshold = 3
"#;

#[test]
fn reaching_within_the_threshold_is_satisfied() {
    let graph = hub_reaching(3, Resolution::Resolved);
    assert_eq!(outcome_for(FAN_OUT_3, &graph).verdict, Verdict::Satisfied);
}

#[test]
fn reaching_past_the_threshold_on_resolved_edges_is_a_violation() {
    let graph = hub_reaching(5, Resolution::Resolved);
    let outcome = outcome_for(FAN_OUT_3, &graph);

    assert_eq!(outcome.verdict, Verdict::Violated);
    let detail = &outcome.violations[0].detail;
    assert!(detail.contains("reaches 5 distinct symbols"), "{detail}");
    assert!(detail.contains("limit is 3"), "{detail}");
}

#[test]
fn weak_edges_that_could_carry_it_over_are_uncertainty_not_a_pass() {
    // The property inherited from fan-in: the count that matters is one
    // nobody can see.
    let graph = hub_reaching(5, Resolution::Structural);
    let outcome = outcome_for(FAN_OUT_3, &graph);

    assert_eq!(outcome.verdict, Verdict::Inconclusive);
    assert!(outcome.uncertainty.expect("uncertainty").unresolved_edges > 0);
}

#[test]
fn fan_out_counts_distinct_targets_not_edges() {
    // The deliberate asymmetry with fan-in, which counts edges. A file that
    // imports the same module on three lines reaches one thing, not three,
    // and inflating that would make the threshold meaningless.
    let hub = symbol("src/hub.ts", "hub", SymbolKind::Function);
    let one = symbol("src/one.ts", "one", SymbolKind::Function);

    let mut repeated: Vec<Relationship> = Vec::new();
    for line in 1..=6 {
        let mut e = edge(&hub, &one, Resolution::Resolved);
        e.line = line;
        repeated.push(e);
    }
    let graph = build(
        &[("src/hub.ts", vec![hub]), ("src/one.ts", vec![one])],
        &repeated,
    );

    assert_eq!(
        outcome_for(FAN_OUT_3, &graph).verdict,
        Verdict::Satisfied,
        "six edges to one target is a fan-out of one"
    );
}

// ── naming-convention ────────────────────────────────────────

const HANDLERS: &str = r#"
[[rule]]
name = "handlers-are-suffixed"
kind = "naming-convention"
symbols = "src/http/**"
matches = "*Handler"
"#;

fn named(file: &str, name: &str, kind: SymbolKind) -> InvarGraph {
    build(&[(file, vec![symbol(file, name, kind)])], &[])
}

#[test]
fn a_matching_name_is_satisfied() {
    let graph = named("src/http/login.ts", "LoginHandler", SymbolKind::Class);
    assert_eq!(outcome_for(HANDLERS, &graph).verdict, Verdict::Satisfied);
}

#[test]
fn a_name_outside_the_pattern_is_a_violation_naming_the_kind() {
    let graph = named("src/http/login.ts", "Login", SymbolKind::Class);
    let outcome = outcome_for(HANDLERS, &graph);

    assert_eq!(outcome.verdict, Verdict::Violated);
    let detail = &outcome.violations[0].detail;
    assert!(detail.contains("class 'Login'"), "{detail}");
    assert!(detail.contains("*Handler"), "{detail}");
}

#[test]
fn a_symbol_outside_the_scope_is_not_judged() {
    let graph = named("src/db/login.ts", "Login", SymbolKind::Class);
    assert_eq!(outcome_for(HANDLERS, &graph).verdict, Verdict::Satisfied);
}

#[test]
fn the_only_filter_narrows_to_one_kind() {
    let rules_text = r#"
[[rule]]
name = "classes-only"
kind = "naming-convention"
symbols = "src/http/**"
matches = "*Handler"
only = "class"
"#;
    // A function named Login is untouched by a rule scoped to classes.
    let graph = named("src/http/login.ts", "Login", SymbolKind::Function);
    assert_eq!(outcome_for(rules_text, &graph).verdict, Verdict::Satisfied);

    let graph = named("src/http/login.ts", "Login", SymbolKind::Class);
    assert_eq!(outcome_for(rules_text, &graph).verdict, Verdict::Violated);
}

#[test]
fn naming_is_never_inconclusive_even_where_nothing_resolves() {
    // The property that makes this kind different. Every other rule reasons
    // about edges, and an edge that did not resolve could always have been the
    // one that mattered. A name comes from the parse, so a graph made entirely
    // of structural edges still gives a definite answer here.
    let a = symbol("src/http/a.ts", "AHandler", SymbolKind::Class);
    let b = symbol("src/http/b.ts", "BHandler", SymbolKind::Class);
    let weak = edge(&a, &b, Resolution::Structural);
    let graph = build(
        &[("src/http/a.ts", vec![a]), ("src/http/b.ts", vec![b])],
        &[weak],
    );

    let outcome = outcome_for(HANDLERS, &graph);
    assert_eq!(outcome.verdict, Verdict::Satisfied);
    assert!(
        outcome.uncertainty.is_none(),
        "a naming rule has no uncertainty to report"
    );
}
