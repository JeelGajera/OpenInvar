//! `requires-test`.
//!
//! The rule no comparable tool can express — `tests` edges are derived across
//! every Tier 1 language here, and semgrep, dependency-cruiser, ArchUnit and
//! import-linter have no equivalent in any language.
//!
//! Two decisions carry the design, and both have a test named for them.
//!
//! Coverage counts **direct** edges only. Static reachability is not
//! execution, and a transitive version would be trivially gameable by the
//! agents this product exists to check.
//!
//! An apparent gap in coverage is **uncertainty, not a violation**, wherever
//! the graph has structural regions: a test in a Tier 2 file produces no edge
//! at all, so "no test reaches this" and "no test this build can see reaches
//! this" are indistinguishable.

use openinvar_core::delta;
use openinvar_core::graph::InvarGraph;
use openinvar_core::incremental::replace_file_ir;
use openinvar_core::ir::{
    FileIR, Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind,
};
use openinvar_core::rule_eval::{evaluate, RuleOutcome, Verdict};
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

fn relationship(
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
            },
        );
    }
    for edge in edges {
        graph.add_relationship(edge);
    }
    graph
}

const COVERED: &str = r#"
[[rule]]
name = "api-is-covered"
kind = "requires-test"
symbols = "src/api/**"
"#;

fn outcome(rules_text: &str, graph: &InvarGraph) -> RuleOutcome {
    let parsed = rules::parse(rules_text).expect("rules parse");
    evaluate(&parsed, graph, None).outcomes.remove(0)
}

#[test]
fn a_symbol_a_test_reaches_is_satisfied() {
    let api = symbol("src/api/login.ts", "login");
    let spec = symbol("src/api/login.test.ts", "it_logs_in");
    let tests = relationship(&spec, &api, RelationshipKind::Tests, Resolution::Resolved);
    let graph = build(
        &[
            ("src/api/login.ts", vec![api]),
            ("src/api/login.test.ts", vec![spec]),
        ],
        &[tests],
    );

    assert_eq!(outcome(COVERED, &graph).verdict, Verdict::Satisfied);
}

#[test]
fn a_symbol_no_test_reaches_is_a_violation() {
    let api = symbol("src/api/login.ts", "login");
    let graph = build(&[("src/api/login.ts", vec![api])], &[]);
    let found = outcome(COVERED, &graph);

    assert_eq!(found.verdict, Verdict::Violated);
    assert!(
        found.violations[0].detail.contains("'login' is not reached by any test"),
        "{}",
        found.violations[0].detail
    );
}

#[test]
fn coverage_does_not_travel_through_another_symbol() {
    // Static reachability is not execution. That the test reaches `login`, and
    // `login` references `format_token`, is no proof the test ever runs
    // `format_token` — it may sit behind an early return.
    //
    // It is also the loophole that would make this rule worthless against the
    // agents it exists to check: wire a new function into any legacy endpoint
    // that already has a test, and a transitive rule goes green without a
    // single assertion being written.
    let api = symbol("src/api/login.ts", "login");
    let helper = symbol("src/api/token.ts", "format_token");
    let spec = symbol("src/api/login.test.ts", "it_logs_in");

    let tests = relationship(&spec, &api, RelationshipKind::Tests, Resolution::Resolved);
    let calls = relationship(&api, &helper, RelationshipKind::Calls, Resolution::Resolved);

    let graph = build(
        &[
            ("src/api/login.ts", vec![api]),
            ("src/api/token.ts", vec![helper]),
            ("src/api/login.test.ts", vec![spec]),
        ],
        &[tests, calls],
    );

    let found = outcome(COVERED, &graph);
    assert_eq!(
        found.verdict,
        Verdict::Violated,
        "format_token is reached only transitively and is not covered"
    );
    assert!(
        found.violations.iter().any(|v| v.detail.contains("format_token")),
        "the transitively-reached symbol must be the one reported"
    );
}

#[test]
fn a_gap_is_uncertainty_where_the_graph_has_structural_regions() {
    // A test in a Tier 2 file produces no `tests` edge at all, so "nothing
    // covers this" and "nothing this build can see covers this" are the same
    // observation. Reporting a violation would be an accusation the evidence
    // cannot support.
    let api = symbol("src/api/login.ts", "login");
    let other = symbol("src/api/other.ts", "other");
    let weak = relationship(&api, &other, RelationshipKind::Calls, Resolution::Structural);

    let graph = build(
        &[
            ("src/api/login.ts", vec![api]),
            ("src/api/other.ts", vec![other]),
        ],
        &[weak],
    );

    let found = outcome(COVERED, &graph);
    assert_eq!(found.verdict, Verdict::Inconclusive);
    assert!(found.uncertainty.expect("uncertainty").unresolved_edges > 0);
}

#[test]
fn a_test_edge_that_did_not_resolve_does_not_count_as_coverage() {
    // A structural `tests` edge was matched by name within one file. It cannot
    // support the claim that this symbol is covered.
    let api = symbol("src/api/login.ts", "login");
    let spec = symbol("src/api/login.test.ts", "it_logs_in");
    let weak = relationship(&spec, &api, RelationshipKind::Tests, Resolution::Structural);
    let graph = build(
        &[
            ("src/api/login.ts", vec![api]),
            ("src/api/login.test.ts", vec![spec]),
        ],
        &[weak],
    );

    assert_ne!(
        outcome(COVERED, &graph).verdict,
        Verdict::Satisfied,
        "a structural tests edge is not proof of coverage"
    );
}

#[test]
fn a_symbol_outside_the_scope_is_not_judged() {
    let internal = symbol("src/internal/helper.ts", "helper");
    let graph = build(&[("src/internal/helper.ts", vec![internal])], &[]);
    assert_eq!(outcome(COVERED, &graph).verdict, Verdict::Satisfied);
}

// ── new_only ─────────────────────────────────────────────────

const NEW_ONLY: &str = r#"
[[rule]]
name = "new-api-is-covered"
kind = "requires-test"
symbols = "src/api/**"
new_only = true
"#;

#[test]
fn new_only_is_skipped_rather_than_passed_without_a_change() {
    // A rule that needs a comparison and did not get one is skipped. Counting
    // it as satisfied would report a conclusion nothing reached.
    let api = symbol("src/api/login.ts", "login");
    let graph = build(&[("src/api/login.ts", vec![api])], &[]);
    let found = outcome(NEW_ONLY, &graph);

    assert_eq!(found.verdict, Verdict::Skipped);
    assert!(found.skipped_because.is_some());
}

#[test]
fn new_only_ignores_symbols_that_were_already_there() {
    // The form that makes this rule adoptable. "Every symbol you added is
    // covered" is enforceable on a repository that could never pass "every
    // symbol is covered" — which is a coverage project, not a gate.
    let old = symbol("src/api/legacy.ts", "legacy");
    let before = build(&[("src/api/legacy.ts", vec![old.clone()])], &[]);
    let after = build(&[("src/api/legacy.ts", vec![old])], &[]);
    let computed = delta::compute(&before, &after);

    let parsed = rules::parse(NEW_ONLY).expect("rules parse");
    let evaluation = evaluate(&parsed, &after, Some(&computed));

    assert_eq!(
        evaluation.outcomes[0].verdict,
        Verdict::Satisfied,
        "an uncovered symbol that predates the change is not this change's problem"
    );
}

#[test]
fn new_only_reports_an_uncovered_symbol_the_change_added() {
    let old = symbol("src/api/legacy.ts", "legacy");
    let before = build(&[("src/api/legacy.ts", vec![old.clone()])], &[]);

    let fresh = symbol("src/api/new.ts", "brand_new");
    let after = build(
        &[
            ("src/api/legacy.ts", vec![old]),
            ("src/api/new.ts", vec![fresh]),
        ],
        &[],
    );
    let computed = delta::compute(&before, &after);

    let parsed = rules::parse(NEW_ONLY).expect("rules parse");
    let evaluation = evaluate(&parsed, &after, Some(&computed));
    let found = &evaluation.outcomes[0];

    assert_eq!(found.verdict, Verdict::Violated);
    assert!(
        found.violations.iter().any(|v| v.detail.contains("brand_new")),
        "the newly added symbol must be the one reported"
    );
}

#[test]
fn a_test_is_not_itself_required_to_be_covered() {
    // Nobody writes a test for a test. A test is recognised by originating a
    // `tests` edge rather than by its filename — `is_test_file` lives in the
    // language crate, which the engine cannot depend on, and the graph already
    // carries the answer.
    let api = symbol("src/api/login.ts", "login");
    let spec = symbol("src/api/login.test.ts", "it_logs_in");
    let tests = relationship(&spec, &api, RelationshipKind::Tests, Resolution::Resolved);

    // Both files are inside the rule's scope, so the test function would be
    // judged too if it were not excluded.
    let graph = build(
        &[
            ("src/api/login.ts", vec![api]),
            ("src/api/login.test.ts", vec![spec]),
        ],
        &[tests],
    );

    let found = outcome(COVERED, &graph);
    assert_eq!(
        found.verdict,
        Verdict::Satisfied,
        "the test function must not be reported as uncovered: {:?}",
        found.violations
    );
}
