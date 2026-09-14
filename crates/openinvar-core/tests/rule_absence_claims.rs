//! `requires-dependency` and `no-orphans`.
//!
//! Both assert that something is *not* there, which is the hard direction: a
//! missing edge and an unresolved edge look identical. They are implemented
//! and tested together so their uncertainty semantics cannot drift apart.
//!
//! `requires-dependency` is `forbid-dependency` inverted, and the inversion is
//! the subtlety — there an unresolved edge might be a violation, here it might
//! be the *satisfying* edge.
//!
//! `no-orphans` is the weakest claim in the vocabulary and ships at `warn`.

use openinvar_core::graph::InvarGraph;
use openinvar_core::incremental::replace_file_ir;
use openinvar_core::ir::{
    FileIR, Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind,
};
use openinvar_core::rule_eval::{evaluate, RuleOutcome, Verdict};
use openinvar_core::rules::{self, Severity};

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

fn relationship(from: &Symbol, to: &Symbol, resolution: Resolution) -> Relationship {
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
            },
        );
    }
    for edge in edges {
        graph.add_relationship(edge);
    }
    graph
}

fn outcome(rules_text: &str, graph: &InvarGraph) -> RuleOutcome {
    let parsed = rules::parse(rules_text).expect("rules parse");
    evaluate(&parsed, graph, None).outcomes.remove(0)
}

const HANDLERS_LOG: &str = r#"
[[rule]]
name = "handlers-log"
kind = "requires-dependency"
from = "src/http/**"
to   = "src/audit/**"
"#;

#[test]
fn a_symbol_that_references_the_target_is_satisfied() {
    let handler = symbol("src/http/login.ts", "login");
    let audit = symbol("src/audit/log.ts", "record");
    let edge = relationship(&handler, &audit, Resolution::Resolved);
    let graph = build(
        &[
            ("src/http/login.ts", vec![handler]),
            ("src/audit/log.ts", vec![audit]),
        ],
        &[edge],
    );

    assert_eq!(outcome(HANDLERS_LOG, &graph).verdict, Verdict::Satisfied);
}

#[test]
fn a_symbol_with_no_qualifying_edge_at_all_is_a_violation() {
    let handler = symbol("src/http/login.ts", "login");
    let graph = build(&[("src/http/login.ts", vec![handler])], &[]);
    let found = outcome(HANDLERS_LOG, &graph);

    assert_eq!(found.verdict, Verdict::Violated);
    assert!(
        found.violations[0].detail.contains("references nothing matching"),
        "{}",
        found.violations[0].detail
    );
}

#[test]
fn an_unresolved_edge_might_be_the_satisfying_one() {
    // The inversion that makes this different from forbid-dependency. There,
    // an unresolved edge might be a violation. Here it might be what satisfies
    // the rule, so reporting a breach would be an accusation the evidence
    // cannot support.
    let handler = symbol("src/http/login.ts", "login");
    let somewhere = symbol("src/other/thing.ts", "thing");
    let weak = relationship(&handler, &somewhere, Resolution::Structural);
    let graph = build(
        &[
            ("src/http/login.ts", vec![handler]),
            ("src/other/thing.ts", vec![somewhere]),
        ],
        &[weak],
    );

    let found = outcome(HANDLERS_LOG, &graph);
    assert_eq!(found.verdict, Verdict::Inconclusive);
    assert!(found.uncertainty.expect("uncertainty").unresolved_edges > 0);
}

#[test]
fn a_symbol_outside_the_from_scope_is_not_required_to_do_anything() {
    let other = symbol("src/db/query.ts", "query");
    let graph = build(&[("src/db/query.ts", vec![other])], &[]);
    assert_eq!(outcome(HANDLERS_LOG, &graph).verdict, Verdict::Satisfied);
}

// ── no-orphans ───────────────────────────────────────────────

const NO_ORPHANS: &str = r#"
[[rule]]
name = "no-dead-code"
kind = "no-orphans"
scope = "src/**"
roots = ["src/main.ts"]
"#;

#[test]
fn a_referenced_symbol_is_not_an_orphan() {
    let entry = symbol("src/main.ts", "main");
    let used = symbol("src/used.ts", "used");
    let edge = relationship(&entry, &used, Resolution::Resolved);
    let graph = build(
        &[
            ("src/main.ts", vec![entry]),
            ("src/used.ts", vec![used]),
        ],
        &[edge],
    );

    assert_eq!(outcome(NO_ORPHANS, &graph).verdict, Verdict::Satisfied);
}

#[test]
fn an_unreferenced_symbol_is_reported() {
    let entry = symbol("src/main.ts", "main");
    let dead = symbol("src/dead.ts", "dead");
    let graph = build(
        &[
            ("src/main.ts", vec![entry]),
            ("src/dead.ts", vec![dead]),
        ],
        &[],
    );

    let found = outcome(NO_ORPHANS, &graph);
    assert_eq!(found.verdict, Verdict::Violated);
    assert!(
        found.violations.iter().any(|v| v.detail.contains("'dead' is referenced by nothing")),
        "{:?}",
        found.violations
    );
}

#[test]
fn a_declared_root_is_never_an_orphan() {
    // An entry point is unreferenced by definition. A rule that reports every
    // `main` in a repository is one nobody keeps switched on.
    let entry = symbol("src/main.ts", "main");
    let graph = build(&[("src/main.ts", vec![entry])], &[]);
    assert_eq!(outcome(NO_ORPHANS, &graph).verdict, Verdict::Satisfied);
}

#[test]
fn an_apparent_orphan_is_uncertainty_where_anything_is_unresolved() {
    // The weakest claim in the vocabulary. Any edge that did not resolve could
    // be the reference that makes this symbol reachable, so an apparent orphan
    // and an orphan this build cannot see referenced are the same observation.
    let entry = symbol("src/main.ts", "main");
    let dead = symbol("src/dead.ts", "dead");
    let other = symbol("src/other.ts", "other");
    let weak = relationship(&entry, &other, Resolution::Structural);
    let graph = build(
        &[
            ("src/main.ts", vec![entry]),
            ("src/dead.ts", vec![dead]),
            ("src/other.ts", vec![other]),
        ],
        &[weak],
    );

    assert_eq!(outcome(NO_ORPHANS, &graph).verdict, Verdict::Inconclusive);
}

#[test]
fn no_orphans_defaults_to_warn_rather_than_blocking() {
    // Unreferenced code is frequently intentional: a feature landing before
    // its callers, a utility written ahead of use. Blocking CI on it would
    // stop ordinary work, and given how often this rule can only say
    // "undecided" anyway, defaulting it to error would be claiming a certainty
    // the graph cannot supply.
    let parsed = rules::parse(NO_ORPHANS).expect("rules parse");
    assert_eq!(parsed.rules[0].severity, Severity::Warn);
}

#[test]
fn requires_dependency_defaults_to_error() {
    // The other side of the split: this one states a policy, not a smell.
    let parsed = rules::parse(HANDLERS_LOG).expect("rules parse");
    assert_eq!(parsed.rules[0].severity, Severity::Error);
}
