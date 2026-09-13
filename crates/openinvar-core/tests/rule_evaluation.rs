//! Evaluating rules against a graph.
//!
//! The case worth most of the attention here is the one a boolean verdict
//! cannot express: a rule whose scope contains edges too weakly resolved to
//! judge. Reporting that as a pass would be reporting a conclusion the
//! analysis never reached, so it has its own verdict and its own tests.

use openinvar_core::delta;
use openinvar_core::graph::InvarGraph;
use openinvar_core::incremental::replace_file_ir;
use openinvar_core::ir::{
    FileIR, Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind,
};
use openinvar_core::rule_eval::{evaluate, Verdict};
use openinvar_core::rules::{self, Severity};

fn symbol(file: &str, name: &str, kind: SymbolKind, lines: (u32, u32)) -> Symbol {
    Symbol {
        id: format!("{file}::{name}::{kind:?}").to_lowercase(),
        name: name.to_string(),
        kind,
        language: Language::TypeScript,
        file: file.to_string(),
        line_start: lines.0,
        line_end: lines.1,
        signature: None,
    }
}

fn edge(
    from: &Symbol,
    to: &Symbol,
    kind: RelationshipKind,
    resolution: Resolution,
) -> Relationship {
    Relationship {
        from: from.id.clone(),
        to: to.id.clone(),
        kind,
        alias: None,
        properties_accessed: vec![],
        context: "test".to_string(),
        file: from.file.clone(),
        line: from.line_start,
        resolution,
    }
}

/// Build a graph from whole files.
///
/// Symbols for every file first, then the edges. `add_relationship` drops an
/// edge whose target is not yet a node, and `replace_file_ir` clears a file
/// before rewriting it — so any single-pass build would silently lose the
/// forward references these rules exist to find.
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
            },
        );
    }
    for (_, _, relationships) in &files {
        for relationship in relationships {
            graph.add_relationship(relationship);
        }
    }
    graph
}

/// A core symbol, a cli symbol, and one edge between them.
fn layered(kind: RelationshipKind, resolution: Resolution) -> InvarGraph {
    let core = symbol("core/engine.ts", "Engine", SymbolKind::Class, (1, 20));
    let cli = symbol("cli/main.ts", "main", SymbolKind::Function, (1, 10));
    let e = edge(&core, &cli, kind, resolution);
    graph_of(vec![
        ("core/engine.ts", vec![core], vec![e]),
        ("cli/main.ts", vec![cli], vec![]),
    ])
}

fn parse(text: &str) -> rules::Rules {
    rules::parse(text).expect("test rules should parse")
}

const LAYERING: &str = r#"
[[rule]]
name = "core-must-not-depend-on-cli"
kind = "forbid-dependency"
from = "core/**"
to = "cli/**"
"#;

// ── violations are reported only on resolved evidence ────────

#[test]
fn a_resolved_forbidden_import_is_a_violation() {
    let graph = layered(RelationshipKind::Imports, Resolution::Resolved);
    let evaluation = evaluate(&parse(LAYERING), &graph, None);

    let outcome = &evaluation.outcomes[0];
    assert_eq!(outcome.verdict, Verdict::Violated);
    assert_eq!(outcome.violations.len(), 1);
    assert_eq!(outcome.violations[0].file, "core/engine.ts");
    assert!(
        outcome.violations[0].detail.contains("cli/main.ts"),
        "the violation should name what was reached: {}",
        outcome.violations[0].detail
    );
    assert!(evaluation.should_fail(), "an error-severity breach fails the gate");
}

#[test]
fn the_same_import_resolved_only_structurally_is_inconclusive_not_a_violation() {
    let graph = layered(RelationshipKind::Imports, Resolution::Structural);
    let evaluation = evaluate(&parse(LAYERING), &graph, None);

    let outcome = &evaluation.outcomes[0];
    assert_eq!(
        outcome.verdict,
        Verdict::Inconclusive,
        "a structural edge names some target, not this one"
    );
    assert!(outcome.violations.is_empty());
    assert_eq!(outcome.uncertainty.as_ref().unwrap().unresolved_edges, 1);
    assert!(
        !evaluation.should_fail(),
        "inconclusive fails open: it is a question, not a breach"
    );
}

#[test]
fn a_clean_scope_with_a_weak_edge_in_it_is_inconclusive_not_satisfied() {
    // The edge points somewhere allowed, but it is weakly resolved, so it
    // cannot be shown to point somewhere allowed. That is the whole point:
    // "no violation found" is not "no violation exists".
    let core = symbol("core/engine.ts", "Engine", SymbolKind::Class, (1, 20));
    let other = symbol("core/util.ts", "helper", SymbolKind::Function, (1, 5));
    let e = edge(
        &core,
        &other,
        RelationshipKind::Imports,
        Resolution::Structural,
    );
    let graph = graph_of(vec![
        ("core/engine.ts", vec![core], vec![e]),
        ("core/util.ts", vec![other], vec![]),
    ]);

    let outcome = &evaluate(&parse(LAYERING), &graph, None).outcomes[0];
    assert_eq!(outcome.verdict, Verdict::Inconclusive);
}

#[test]
fn a_fully_resolved_clean_scope_is_satisfied() {
    let core = symbol("core/engine.ts", "Engine", SymbolKind::Class, (1, 20));
    let other = symbol("core/util.ts", "helper", SymbolKind::Function, (1, 5));
    let e = edge(&core, &other, RelationshipKind::Imports, Resolution::Resolved);
    let graph = graph_of(vec![
        ("core/engine.ts", vec![core], vec![e]),
        ("core/util.ts", vec![other], vec![]),
    ]);

    let outcome = &evaluate(&parse(LAYERING), &graph, None).outcomes[0];
    assert_eq!(
        outcome.verdict,
        Verdict::Satisfied,
        "every edge in scope was examined, so the absence claim is earned"
    );
    assert!(outcome.uncertainty.is_none());
}

#[test]
fn a_violation_outranks_uncertainty_elsewhere_in_the_same_scope() {
    let core = symbol("core/engine.ts", "Engine", SymbolKind::Class, (1, 20));
    let vague = symbol("core/vague.ts", "Vague", SymbolKind::Class, (1, 9));
    let cli = symbol("cli/main.ts", "main", SymbolKind::Function, (1, 10));
    let resolved = edge(&core, &cli, RelationshipKind::Imports, Resolution::Resolved);
    let weak = edge(
        &vague,
        &cli,
        RelationshipKind::Imports,
        Resolution::Structural,
    );
    let graph = graph_of(vec![
        ("core/engine.ts", vec![core], vec![resolved]),
        ("core/vague.ts", vec![vague], vec![weak]),
        ("cli/main.ts", vec![cli], vec![]),
    ]);

    let outcome = &evaluate(&parse(LAYERING), &graph, None).outcomes[0];
    assert_eq!(
        outcome.verdict,
        Verdict::Violated,
        "weak evidence elsewhere does not soften a breach found on strong evidence"
    );
}

// ── dependency is narrower than reference ────────────────────

#[test]
fn forbid_dependency_ignores_a_call_that_forbid_reference_catches() {
    let graph = layered(RelationshipKind::Calls, Resolution::Resolved);

    let dependency = &evaluate(&parse(LAYERING), &graph, None).outcomes[0];
    assert_eq!(
        dependency.verdict,
        Verdict::Satisfied,
        "a call is not a dependency; 'you may call in, but not import' must be expressible"
    );

    let reference = &evaluate(
        &parse(
            r#"
[[rule]]
name = "no-reference"
kind = "forbid-reference"
from = "core/**"
to = "cli/**"
"#,
        ),
        &graph,
        None,
    )
    .outcomes[0];
    assert_eq!(reference.verdict, Verdict::Violated);
}

// ── severity ─────────────────────────────────────────────────

#[test]
fn a_warn_severity_breach_is_reported_but_does_not_fail_the_gate() {
    let graph = layered(RelationshipKind::Imports, Resolution::Resolved);
    let evaluation = evaluate(
        &parse(
            r#"
[[rule]]
name = "advisory"
kind = "forbid-dependency"
from = "core/**"
to = "cli/**"
severity = "warn"
"#,
        ),
        &graph,
        None,
    );

    assert_eq!(evaluation.outcomes[0].verdict, Verdict::Violated);
    assert_eq!(evaluation.outcomes[0].severity, Severity::Warn);
    assert!(!evaluation.should_fail());
    assert_eq!(evaluation.blocking().count(), 0);
}

// ── max-fan-in ───────────────────────────────────────────────

fn fan_in_graph(resolved: usize, structural: usize) -> InvarGraph {
    let hub = symbol("core/hub.ts", "Hub", SymbolKind::Class, (1, 5));
    let mut files: Vec<(String, Vec<Symbol>, Vec<Relationship>)> = vec![(
        "core/hub.ts".to_string(),
        vec![hub.clone()],
        vec![],
    )];
    for i in 0..(resolved + structural) {
        let file = format!("app/caller{i}.ts");
        let caller = symbol(&file, &format!("caller{i}"), SymbolKind::Function, (1, 2));
        let resolution = if i < resolved {
            Resolution::Resolved
        } else {
            Resolution::Structural
        };
        let e = edge(&caller, &hub, RelationshipKind::Calls, resolution);
        files.push((file, vec![caller], vec![e]));
    }
    graph_of(
        files
            .iter()
            .map(|(f, s, r)| (f.as_str(), s.clone(), r.clone()))
            .collect(),
    )
}

const FAN_IN: &str = r#"
[[rule]]
name = "god-node"
kind = "max-fan-in"
threshold = 3
"#;

#[test]
fn resolved_fan_in_over_the_threshold_is_a_violation() {
    let outcome = &evaluate(&parse(FAN_IN), &fan_in_graph(4, 0), None).outcomes[0];
    assert_eq!(outcome.verdict, Verdict::Violated);
    assert_eq!(outcome.violations.len(), 1);
    assert!(outcome.violations[0].detail.contains('4'));
}

#[test]
fn resolved_fan_in_within_the_threshold_is_satisfied() {
    let outcome = &evaluate(&parse(FAN_IN), &fan_in_graph(3, 0), None).outcomes[0];
    assert_eq!(outcome.verdict, Verdict::Satisfied);
}

#[test]
fn weak_edges_that_could_carry_a_symbol_over_the_threshold_are_inconclusive() {
    // Two resolved, two structural. The real count is somewhere in 2..=4 and
    // the threshold is 3, so nobody can say whether this passes.
    let outcome = &evaluate(&parse(FAN_IN), &fan_in_graph(2, 2), None).outcomes[0];
    assert_eq!(outcome.verdict, Verdict::Inconclusive);
    assert_eq!(outcome.uncertainty.as_ref().unwrap().unresolved_edges, 2);
}

#[test]
fn weak_edges_that_cannot_reach_the_threshold_leave_the_rule_satisfied() {
    // One resolved, one structural: at most two, threshold is three. The
    // uncertainty exists but cannot change the answer, so it is not raised.
    let outcome = &evaluate(&parse(FAN_IN), &fan_in_graph(1, 1), None).outcomes[0];
    assert_eq!(
        outcome.verdict,
        Verdict::Satisfied,
        "uncertainty that cannot change the verdict should not be reported as uncertainty"
    );
}

// ── no-field-removal ─────────────────────────────────────────

const NO_FIELD_REMOVAL: &str = r#"
[[rule]]
name = "config-is-stable"
kind = "no-field-removal"
symbol = "Config"
"#;

fn config_graph(fields: &[&str]) -> InvarGraph {
    let owner = symbol("core/config.ts", "Config", SymbolKind::Class, (1, 20));
    let mut symbols = vec![owner];
    for (i, field) in fields.iter().enumerate() {
        symbols.push(symbol(
            "core/config.ts",
            field,
            SymbolKind::Property,
            (2 + i as u32, 2 + i as u32),
        ));
    }
    graph_of(vec![("core/config.ts", symbols, vec![])])
}

#[test]
fn removing_a_field_from_the_named_symbol_is_a_violation() {
    let before = config_graph(&["host", "port"]);
    let after = config_graph(&["host"]);
    let d = delta::compute(&before, &after);

    let outcome = &evaluate(&parse(NO_FIELD_REMOVAL), &before, Some(&d)).outcomes[0];
    assert_eq!(outcome.verdict, Verdict::Violated);
    assert_eq!(outcome.violations.len(), 1);
    assert!(outcome.violations[0].detail.contains("port"));
}

#[test]
fn adding_a_field_is_not_a_violation() {
    let before = config_graph(&["host"]);
    let after = config_graph(&["host", "port"]);
    let d = delta::compute(&before, &after);

    let outcome = &evaluate(&parse(NO_FIELD_REMOVAL), &before, Some(&d)).outcomes[0];
    assert_eq!(outcome.verdict, Verdict::Satisfied);
}

#[test]
fn a_field_removed_from_a_symbol_the_rule_does_not_name_is_ignored() {
    let owner = symbol("core/other.ts", "Other", SymbolKind::Class, (1, 20));
    let field = symbol("core/other.ts", "gone", SymbolKind::Property, (2, 2));
    let before = graph_of(vec![("core/other.ts", vec![owner.clone(), field], vec![])]);
    let after = graph_of(vec![("core/other.ts", vec![owner], vec![])]);
    let d = delta::compute(&before, &after);

    let outcome = &evaluate(&parse(NO_FIELD_REMOVAL), &before, Some(&d)).outcomes[0];
    assert_eq!(outcome.verdict, Verdict::Satisfied);
}

#[test]
fn deleting_the_owner_outright_does_not_report_each_of_its_fields() {
    // The symbol is gone. Reporting two field removals would bury the one
    // fact that matters, and `findings` already reports the removal itself.
    let before = config_graph(&["host", "port"]);
    let after = graph_of(vec![("core/config.ts", vec![], vec![])]);
    let d = delta::compute(&before, &after);

    let outcome = &evaluate(&parse(NO_FIELD_REMOVAL), &before, Some(&d)).outcomes[0];
    assert_eq!(outcome.verdict, Verdict::Satisfied);
}

#[test]
fn a_field_is_attributed_when_the_owner_records_only_its_declaration_line() {
    // The shape TypeScript actually produces: the class records line_end ==
    // line_start, so exact containment finds nothing and the field has to be
    // attributed to the nearest container declared above it. Locked as a test
    // because the first version of this rule reported nothing at all against a
    // real repository while passing every synthetic case.
    let owner = symbol("core/config.ts", "Config", SymbolKind::Class, (1, 1));
    let keep = symbol("core/config.ts", "host", SymbolKind::Property, (2, 2));
    let gone = symbol("core/config.ts", "port", SymbolKind::Property, (3, 3));

    let before = graph_of(vec![(
        "core/config.ts",
        vec![owner.clone(), keep.clone(), gone],
        vec![],
    )]);
    let after = graph_of(vec![("core/config.ts", vec![owner, keep], vec![])]);
    let d = delta::compute(&before, &after);

    let outcome = &evaluate(&parse(NO_FIELD_REMOVAL), &before, Some(&d)).outcomes[0];
    assert_eq!(outcome.verdict, Verdict::Violated);
    assert!(outcome.violations[0].detail.contains("port"));
}

#[test]
fn a_field_of_a_nested_class_is_not_attributed_to_the_enclosing_one() {
    // Where ranges are real, the innermost enclosing container wins.
    let outer = symbol("core/config.ts", "Config", SymbolKind::Class, (1, 20));
    let inner = symbol("core/config.ts", "Inner", SymbolKind::Class, (5, 10));
    let gone = symbol("core/config.ts", "secret", SymbolKind::Property, (6, 6));

    let before = graph_of(vec![(
        "core/config.ts",
        vec![outer.clone(), inner.clone(), gone],
        vec![],
    )]);
    let after = graph_of(vec![("core/config.ts", vec![outer, inner], vec![])]);
    let d = delta::compute(&before, &after);

    let outcome = &evaluate(&parse(NO_FIELD_REMOVAL), &before, Some(&d)).outcomes[0];
    assert_eq!(
        outcome.verdict,
        Verdict::Satisfied,
        "the field belongs to Inner, and the rule names Config"
    );
}

#[test]
fn a_delta_rule_without_a_delta_is_skipped_not_satisfied() {
    let graph = config_graph(&["host"]);
    let evaluation = evaluate(&parse(NO_FIELD_REMOVAL), &graph, None);

    let outcome = &evaluation.outcomes[0];
    assert_eq!(
        outcome.verdict,
        Verdict::Skipped,
        "not looking is not the same as looking and finding nothing"
    );
    assert!(outcome.skipped_because.is_some());
    assert!(!evaluation.should_fail());
}

// ── determinism ──────────────────────────────────────────────

#[test]
fn the_same_graph_evaluates_identically_every_time() {
    let text = format!("{LAYERING}\n{FAN_IN}");
    let parsed = parse(&text);
    let graph = fan_in_graph(5, 2);

    let first = evaluate(&parsed, &graph, None);
    for _ in 0..8 {
        assert_eq!(
            evaluate(&parsed, &graph, None),
            first,
            "evaluation must not depend on hash iteration order"
        );
    }
}

#[test]
fn outcomes_follow_the_order_the_rules_were_written_in() {
    let parsed = parse(&format!("{FAN_IN}\n{LAYERING}"));
    let evaluation = evaluate(&parsed, &fan_in_graph(1, 0), None);

    let names: Vec<&str> = evaluation
        .outcomes
        .iter()
        .map(|o| o.rule.as_str())
        .collect();
    assert_eq!(names, vec!["god-node", "core-must-not-depend-on-cli"]);
}
