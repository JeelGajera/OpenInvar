//! `assertion-removal`: a test that still covers a symbol but stopped checking it.
//!
//! This detector accuses someone of gutting a test while leaving it green, so
//! the negative cases carry more weight than the positive one. Four of the
//! seven below are ordinary work that must stay silent, and two are about
//! refusing to conclude from evidence this build does not have.

use std::collections::BTreeSet;

use openinvar_core::audit::detectors::AssertionRemoval;
use openinvar_core::audit::{AuditContext, Detector, Finding};
use openinvar_core::delta;
use openinvar_core::graph::InvarGraph;
use openinvar_core::incremental::replace_file_ir;
use openinvar_core::ir::{
    FileIR, Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind,
};

const SOURCE: &str = "src/a.ts";
const TEST: &str = "src/a.test.ts";

fn symbol(file: &str, name: &str, signature: &str) -> Symbol {
    Symbol {
        id: format!("{file}::{name}::function"),
        name: name.to_string(),
        kind: SymbolKind::Function,
        language: Language::TypeScript,
        file: file.to_string(),
        line_start: 10,
        line_end: 20,
        signature: Some(signature.to_string()),
    }
}

fn tests_edge(from: &Symbol, to: &Symbol) -> Relationship {
    Relationship {
        from: from.id.clone(),
        to: to.id.clone(),
        kind: RelationshipKind::Tests,
        alias: None,
        properties_accessed: vec![],
        context: "t".to_string(),
        file: from.file.clone(),
        line: from.line_start,
        resolution: Resolution::Resolved,
    }
}

/// One revision: a target with the given signature, and a test covering it
/// with `assertions` assertions.
///
/// `counted` is the flag an adapter sets for a language it can read. Passing
/// false models a language this build cannot count, which must never produce a
/// finding however the numbers move.
fn revision(signature: &str, assertions: u32, covers: bool, counted: bool) -> InvarGraph {
    let target = symbol(SOURCE, "target", signature);
    let test = symbol(TEST, "covers_target", "fn covers_target()");

    let mut graph = InvarGraph::new();
    replace_file_ir(
        &mut graph,
        &FileIR {
            file: SOURCE.to_string(),
            language: Language::TypeScript,
            symbols: vec![target.clone()],
            relationships: vec![],
            diagnostics: vec![],
            re_exports: vec![],
            assertions: Default::default(),
            assertions_counted: counted,
        },
    );

    let mut counts = std::collections::BTreeMap::new();
    if assertions > 0 {
        counts.insert(test.id.clone(), assertions);
    }
    replace_file_ir(
        &mut graph,
        &FileIR {
            file: TEST.to_string(),
            language: Language::TypeScript,
            symbols: vec![test.clone()],
            relationships: vec![],
            diagnostics: vec![],
            re_exports: vec![],
            assertions: counts,
            assertions_counted: counted,
        },
    );

    if covers {
        graph.add_relationship(&tests_edge(&test, &target));
    }
    graph
}

fn run(before: &InvarGraph, after: &InvarGraph) -> Vec<Finding> {
    let computed = delta::compute(before, after);
    let in_scope: BTreeSet<String> = [SOURCE, TEST].iter().map(|f| f.to_string()).collect();
    let ctx = AuditContext {
        before,
        after,
        delta: &computed,
        tier_one_files: &in_scope,
    };
    AssertionRemoval
        .detect(&ctx)
        .into_iter()
        .filter(|f| ctx.in_scope(&f.file))
        .collect()
}

#[test]
fn a_test_that_keeps_coverage_but_loses_assertions_is_reported() {
    // The shape the detector exists for: the suite still passes, the coverage
    // graph is unchanged, and the test no longer checks the thing that moved.
    let before = revision("fn target(a: i32)", 3, true, true);
    let after = revision("fn target(a: i32, b: i32)", 1, true, true);

    let findings = run(&before, &after);
    assert_eq!(findings.len(), 1, "expected one finding: {findings:?}");
    assert!(
        findings[0].summary.contains("lost 2 assertion(s)"),
        "the summary does not say how many were lost: {}",
        findings[0].summary
    );
}

#[test]
fn an_unchanged_target_is_not_a_finding() {
    // Deleting a redundant assertion is ordinary work. Only assertions
    // disappearing *where behaviour changed* is the suspicious combination.
    let before = revision("fn target(a: i32)", 3, true, true);
    let after = revision("fn target(a: i32)", 1, true, true);

    assert!(
        run(&before, &after).is_empty(),
        "tidying a test was reported as tampering"
    );
}

#[test]
fn adding_assertions_alongside_a_change_is_not_a_finding() {
    let before = revision("fn target(a: i32)", 1, true, true);
    let after = revision("fn target(a: i32, b: i32)", 4, true, true);

    assert!(
        run(&before, &after).is_empty(),
        "strengthening a test was reported"
    );
}

#[test]
fn keeping_the_same_count_is_not_a_finding() {
    let before = revision("fn target(a: i32)", 2, true, true);
    let after = revision("fn target(a: i32, b: i32)", 2, true, true);

    assert!(
        run(&before, &after).is_empty(),
        "an unchanged assertion count was reported"
    );
}

#[test]
fn coverage_that_disappeared_belongs_to_test_tampering_not_here() {
    // Both detectors would otherwise fire on one event and say the same thing
    // twice. Losing the `tests` edge is test-tampering's finding.
    let before = revision("fn target(a: i32)", 3, true, true);
    let after = revision("fn target(a: i32, b: i32)", 0, false, true);

    assert!(
        run(&before, &after).is_empty(),
        "reported a finding that test-tampering already covers"
    );
}

#[test]
fn an_uncounted_language_is_never_a_finding() {
    // Zero assertions in a language this build cannot read means "unknown",
    // not "none". Without this gate every Tier 2 test in a repository would be
    // accused at once.
    let before = revision("fn target(a: i32)", 3, true, false);
    let after = revision("fn target(a: i32, b: i32)", 0, true, false);

    assert!(
        run(&before, &after).is_empty(),
        "accused a test in a language whose assertions cannot be seen"
    );
}

#[test]
fn a_head_that_cannot_count_is_never_a_finding() {
    // The asymmetric case, and the one the gate actually earns its place on: a
    // base that counted, compared against a head that could not. The numbers
    // then fall from three to zero purely because the second build could not
    // see them, and without the gate that reads as a gutted test.
    //
    // A base written before the format carried counts is the mirror of this and
    // is safe on its own — an uncounted base reads as zero, and a count can
    // only rise from zero — so this direction is the one worth pinning.
    let before = revision("fn target(a: i32)", 3, true, true);
    let after = revision("fn target(a: i32, b: i32)", 0, true, false);

    assert!(
        run(&before, &after).is_empty(),
        "a head that cannot count assertions was read as a test losing them"
    );
}
