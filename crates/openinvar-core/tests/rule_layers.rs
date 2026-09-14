//! `layers` and `independence`.
//!
//! Both are scope-set rules over the same edges `forbid-dependency` walks, and
//! both are evaluated natively rather than desugared into pairwise forbids.
//! The test that justifies that decision is
//! `the_top_layer_is_never_uncertain`: the outermost layer may depend on
//! everything, so its weakly resolved edges cannot be upward violations and
//! must not drag the rule to inconclusive. A pairwise desugaring has no way to
//! know that.

use openinvar_core::graph::InvarGraph;
use openinvar_core::incremental::replace_file_ir;
use openinvar_core::ir::{
    FileIR, Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind,
};
use openinvar_core::rule_eval::{evaluate, Verdict};
use openinvar_core::rules;

fn symbol(file: &str, name: &str) -> Symbol {
    Symbol {
        id: format!("{file}::{name}"),
        name: name.to_string(),
        kind: SymbolKind::Function,
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

/// One edge between two files, each its own symbol.
fn graph_with(from_file: &str, to_file: &str, resolution: Resolution) -> InvarGraph {
    let from = symbol(from_file, "a");
    let to = symbol(to_file, "b");
    let relationship = edge(&from, &to, resolution);

    let mut graph = InvarGraph::new();
    for (file, sym) in [(from_file, &from), (to_file, &to)] {
        replace_file_ir(
            &mut graph,
            &FileIR {
                file: file.to_string(),
                language: Language::TypeScript,
                symbols: vec![sym.clone()],
                relationships: vec![],
                diagnostics: vec![],
                re_exports: vec![],
                assertions: Default::default(),
                assertions_counted: false,
            },
        );
    }
    graph.add_relationship(&relationship);
    graph
}

const STACK: &str = r#"
[[rule]]
name = "architecture-layers"
kind = "layers"
layers = ["cli/**", "service/**", "core/**"]
"#;

fn verdict_for(rules_text: &str, graph: &InvarGraph) -> Verdict {
    let parsed = rules::parse(rules_text).expect("rules parse");
    let evaluation = evaluate(&parsed, graph, None);
    evaluation.outcomes[0].verdict
}

#[test]
fn depending_downward_is_allowed() {
    let graph = graph_with("cli/main.ts", "core/engine.ts", Resolution::Resolved);
    assert_eq!(verdict_for(STACK, &graph), Verdict::Satisfied);
}

#[test]
fn depending_upward_is_a_violation_naming_both_layers() {
    let graph = graph_with("core/engine.ts", "cli/main.ts", Resolution::Resolved);
    let parsed = rules::parse(STACK).expect("rules parse");
    let evaluation = evaluate(&parsed, &graph, None);
    let outcome = &evaluation.outcomes[0];

    assert_eq!(outcome.verdict, Verdict::Violated);
    let detail = &outcome.violations[0].detail;
    // The ordinal is the point of evaluating natively — it is what tells a
    // reader which way the arrow was supposed to point.
    assert!(detail.contains("layer 2"), "{detail}");
    assert!(detail.contains("layer 0"), "{detail}");
    assert!(detail.contains("upward"), "{detail}");
}

#[test]
fn a_sibling_inside_one_layer_is_allowed() {
    // A layer is a boundary against reaching up, not against cohesion inside
    // itself.
    let graph = graph_with("core/engine.ts", "core/parser.ts", Resolution::Resolved);
    assert_eq!(verdict_for(STACK, &graph), Verdict::Satisfied);
}

#[test]
fn the_top_layer_is_never_uncertain() {
    // The reason `layers` is not desugared into pairwise forbids.
    //
    // `cli/**` is the outermost layer, so it may depend on everything. An
    // unresolved edge leaving it cannot be an upward violation, because there
    // is nothing above it to violate. Counting it as uncertainty would report
    // the rule undecided on evidence that could never have mattered — and a
    // pairwise evaluator, which only sees "some scope had a weak edge", has no
    // way to know that.
    let graph = graph_with("cli/main.ts", "core/engine.ts", Resolution::Structural);

    assert_eq!(
        verdict_for(STACK, &graph),
        Verdict::Satisfied,
        "a weak edge out of the top layer must not make the rule undecided"
    );
}

#[test]
fn a_weak_edge_below_the_top_layer_is_uncertain() {
    // The other half of the same argument. `core/**` has two layers above it,
    // so an edge it cannot resolve could be pointing at either of them.
    let graph = graph_with("core/engine.ts", "core/parser.ts", Resolution::Structural);
    let parsed = rules::parse(STACK).expect("rules parse");
    let evaluation = evaluate(&parsed, &graph, None);
    let outcome = &evaluation.outcomes[0];

    assert_eq!(outcome.verdict, Verdict::Inconclusive);
    assert_eq!(
        outcome.uncertainty.as_ref().expect("uncertainty").unresolved_edges,
        1
    );
}

#[test]
fn a_file_in_no_layer_is_outside_the_rule() {
    // Rules are scoped. A path no glob claims is not silently assigned to the
    // bottom layer, which would make every unlisted file a violation waiting
    // to happen.
    let graph = graph_with("scripts/build.ts", "cli/main.ts", Resolution::Resolved);
    assert_eq!(verdict_for(STACK, &graph), Verdict::Satisfied);
}

// ── independence ─────────────────────────────────────────────

const SIBLINGS: &str = r#"
[[rule]]
name = "features-do-not-talk"
kind = "independence"
modules = ["src/billing/**", "src/search/**", "src/inbox/**"]
"#;

#[test]
fn siblings_referencing_each_other_is_a_violation_in_either_direction() {
    for (from, to) in [
        ("src/billing/charge.ts", "src/search/index.ts"),
        ("src/search/index.ts", "src/billing/charge.ts"),
    ] {
        let graph = graph_with(from, to, Resolution::Resolved);
        let parsed = rules::parse(SIBLINGS).expect("rules parse");
        let evaluation = evaluate(&parsed, &graph, None);
        let outcome = &evaluation.outcomes[0];

        assert_eq!(
            outcome.verdict,
            Verdict::Violated,
            "independence has no permitted direction: {from} -> {to}"
        );
        assert!(
            outcome.violations[0].detail.contains("independent"),
            "{}",
            outcome.violations[0].detail
        );
    }
}

#[test]
fn a_module_referencing_itself_is_allowed() {
    let graph = graph_with("src/billing/charge.ts", "src/billing/tax.ts", Resolution::Resolved);
    assert_eq!(verdict_for(SIBLINGS, &graph), Verdict::Satisfied);
}

#[test]
fn a_module_reaching_outside_every_listed_module_is_allowed() {
    // Independence is about these modules and each other. Shared code they all
    // depend on is the intended escape hatch.
    let graph = graph_with("src/billing/charge.ts", "src/shared/money.ts", Resolution::Resolved);
    assert_eq!(verdict_for(SIBLINGS, &graph), Verdict::Satisfied);
}

#[test]
fn every_module_carries_uncertainty_unlike_layers() {
    // No permitted direction means no module is exempt: a weak edge leaving
    // any of them could land in any other.
    let graph = graph_with("src/billing/charge.ts", "src/billing/tax.ts", Resolution::Structural);
    assert_eq!(verdict_for(SIBLINGS, &graph), Verdict::Inconclusive);
}

#[test]
fn uncertainty_is_counted_once_per_edge_not_once_per_pair() {
    // Three modules would give six ordered pairs. Counting the same edge once
    // per pair would report the evidence as six times weaker than it is.
    let graph = graph_with("src/billing/charge.ts", "src/billing/tax.ts", Resolution::Structural);
    let parsed = rules::parse(SIBLINGS).expect("rules parse");
    let evaluation = evaluate(&parsed, &graph, None);

    assert_eq!(
        evaluation.outcomes[0]
            .uncertainty
            .as_ref()
            .expect("uncertainty")
            .unresolved_edges,
        1
    );
}
