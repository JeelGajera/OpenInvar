//! The detectors, each with the case it must catch and the case it must not.
//!
//! Precision is the whole product here. A gate that cries wolf is switched off
//! in week two, and a switched-off gate catches nothing — so for every
//! detector the negative case carries at least as much weight as the positive
//! one. The negatives below are all *ordinary work*: changing a function and
//! its test together, deleting a caller along with its callee, adding a new
//! entry point nothing calls yet. None of those may raise a finding.

use std::collections::BTreeSet;

use openinvar_core::audit::detectors::{self, ContractErosion, DeadOnArrival, TestTampering};
use openinvar_core::audit::{AuditContext, Detector};
use openinvar_core::delta;
use openinvar_core::graph::InvarGraph;
use openinvar_core::incremental::replace_file_ir;
use openinvar_core::ir::{
    FileIR, Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind,
};

fn symbol(file: &str, name: &str, line: u32) -> Symbol {
    Symbol {
        id: format!("{file}::{name}::function"),
        name: name.to_string(),
        kind: SymbolKind::Function,
        language: Language::TypeScript,
        file: file.to_string(),
        line_start: line,
        line_end: line + 2,
        signature: Some(format!("fn {name}()")),
    }
}

fn edge(from: &Symbol, to: &Symbol, kind: RelationshipKind) -> Relationship {
    Relationship {
        from: from.id.clone(),
        to: to.id.clone(),
        kind,
        alias: None,
        properties_accessed: vec![],
        context: "t".to_string(),
        file: from.file.clone(),
        line: from.line_start,
        resolution: Resolution::Resolved,
    }
}

type File<'a> = (&'a str, Vec<Symbol>, Vec<Relationship>);

fn graph_of(files: Vec<File<'_>>) -> InvarGraph {
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
                references_counted: false,
            },
        );
    }
    for (_, _, rels) in &files {
        for r in rels {
            graph.add_relationship(r);
        }
    }
    graph
}

fn scope(files: &[&str]) -> BTreeSet<String> {
    files.iter().map(|f| f.to_string()).collect()
}

/// Run one detector over a before/after pair.
fn run(
    detector: &dyn Detector,
    before: &InvarGraph,
    after: &InvarGraph,
    files: &[&str],
) -> Vec<openinvar_core::audit::Finding> {
    let computed = delta::compute(before, after);
    let in_scope = scope(files);
    let ctx = AuditContext {
        before,
        after,
        delta: &computed,
        tier_one_files: &in_scope,
    };
    detector
        .detect(&ctx)
        .into_iter()
        .filter(|f| ctx.in_scope(&f.file))
        .collect()
}

const FILES: &[&str] = &["src/a.ts", "src/a.test.ts", "src/b.ts"];

// ── test tampering ───────────────────────────────────────────

/// `target` covered by `covers_target`.
///
/// `signature` is what makes the target "change": a symbol's id does not
/// encode its position, so moving it down the file is deliberately not a
/// change OpenInvar reports. Altering the signature is.
fn covered(signature: &str, with_coverage: bool) -> InvarGraph {
    let mut target = symbol("src/a.ts", "target", 10);
    target.signature = Some(signature.to_string());
    let test = symbol("src/a.test.ts", "covers_target", 1);
    let mut rels = vec![edge(&test, &target, RelationshipKind::Calls)];
    if with_coverage {
        rels.push(edge(&test, &target, RelationshipKind::Tests));
    }
    graph_of(vec![
        ("src/a.ts", vec![target], vec![]),
        ("src/a.test.ts", vec![test], rels),
    ])
}

#[test]
fn positive_coverage_removed_from_a_symbol_that_changed() {
    // The shape reward-hacking makes: the test stopped covering the thing, in
    // the same change that altered it.
    let before = covered("fn target(a: i32)", true);
    let after = covered("fn target(a: i32, b: i32)", false);
    let findings = run(&TestTampering, &before, &after, FILES);

    assert_eq!(findings.len(), 1, "{findings:#?}");
    assert_eq!(findings[0].detector, "test-tampering");
    assert!(findings[0].summary.contains("stopped covering"));
    assert_eq!(findings[0].evidence.len(), 2, "an accusation carries its evidence");
}

#[test]
fn negative_changing_a_function_and_its_test_together_is_not_tampering() {
    // The most ordinary thing a developer does. A detector that fires here is
    // one nobody keeps switched on — which is why this is not the literal
    // "test file modified alongside the code" rule.
    let before = covered("fn target(a: i32)", true);
    let after = covered("fn target(a: i32, b: i32)", true);
    let findings = run(&TestTampering, &before, &after, FILES);

    assert!(findings.is_empty(), "ordinary work was reported: {findings:#?}");
}

#[test]
fn negative_deleting_a_symbol_and_its_test_together_is_coherent() {
    // The symbol is gone, so its coverage going too is a coherent removal
    // rather than a test being quietly defanged.
    let before = covered("fn target(a: i32)", true);
    let after = graph_of(vec![("src/a.ts", vec![], vec![]), ("src/a.test.ts", vec![], vec![])]);
    let findings = run(&TestTampering, &before, &after, FILES);

    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn negative_removing_coverage_from_a_symbol_that_did_not_change() {
    // Deleting a redundant test is not tampering when the code it covered is
    // untouched.
    let before = covered("fn target(a: i32)", true);
    let after = covered("fn target(a: i32)", false);
    let findings = run(&TestTampering, &before, &after, FILES);

    assert!(findings.is_empty(), "{findings:#?}");
}

// ── contract erosion ─────────────────────────────────────────

fn caller_and_callee(with_callee: bool, with_caller: bool) -> InvarGraph {
    let callee = symbol("src/a.ts", "callee", 1);
    let caller = symbol("src/b.ts", "caller", 1);
    let mut files: Vec<File<'_>> = Vec::new();
    files.push((
        "src/a.ts",
        if with_callee { vec![callee.clone()] } else { vec![] },
        vec![],
    ));
    files.push((
        "src/b.ts",
        if with_caller { vec![caller.clone()] } else { vec![] },
        if with_caller && with_callee {
            vec![edge(&caller, &callee, RelationshipKind::Calls)]
        } else {
            vec![]
        },
    ));
    graph_of(files)
}

#[test]
fn positive_a_callee_removed_while_its_caller_survives() {
    let before = caller_and_callee(true, true);
    let after = caller_and_callee(false, true);
    let findings = run(&ContractErosion, &before, &after, FILES);

    assert_eq!(findings.len(), 1, "{findings:#?}");
    assert_eq!(findings[0].detector, "contract-erosion");
    assert!(findings[0].summary.contains("still point at it"));
}

#[test]
fn negative_removing_a_caller_and_its_callee_together() {
    // A coherent deletion. Nothing is left broken, so nothing is eroded.
    let before = caller_and_callee(true, true);
    let after = caller_and_callee(false, false);
    let findings = run(&ContractErosion, &before, &after, FILES);

    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn negative_removing_a_symbol_nothing_referred_to() {
    let before = caller_and_callee(true, false);
    let after = caller_and_callee(false, false);
    let findings = run(&ContractErosion, &before, &after, FILES);

    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn negative_a_test_losing_its_subject_is_not_contract_erosion() {
    // That event belongs to test-tampering. Reporting it here as well would
    // raise two accusations for one thing and double every count.
    let target = symbol("src/a.ts", "target", 1);
    let test = symbol("src/a.test.ts", "covers_target", 1);
    let before = graph_of(vec![
        ("src/a.ts", vec![target.clone()], vec![]),
        (
            "src/a.test.ts",
            vec![test.clone()],
            vec![edge(&test, &target, RelationshipKind::Tests)],
        ),
    ]);
    let after = graph_of(vec![
        ("src/a.ts", vec![], vec![]),
        ("src/a.test.ts", vec![test], vec![]),
    ]);
    let findings = run(&ContractErosion, &before, &after, FILES);

    assert!(findings.is_empty(), "{findings:#?}");
}

// ── dead on arrival ──────────────────────────────────────────

#[test]
fn positive_a_new_symbol_only_a_test_refers_to() {
    let before = graph_of(vec![("src/a.ts", vec![], vec![]), ("src/a.test.ts", vec![], vec![])]);
    let helper = symbol("src/a.ts", "onlyForTests", 1);
    let test = symbol("src/a.test.ts", "uses_helper", 1);
    let after = graph_of(vec![
        ("src/a.ts", vec![helper.clone()], vec![]),
        (
            "src/a.test.ts",
            vec![test.clone()],
            vec![
                edge(&test, &helper, RelationshipKind::Calls),
                edge(&test, &helper, RelationshipKind::Tests),
            ],
        ),
    ]);
    let findings = run(&DeadOnArrival, &before, &after, FILES);

    assert_eq!(findings.len(), 1, "{findings:#?}");
    assert_eq!(findings[0].detector, "dead-on-arrival");
}

#[test]
fn negative_a_new_symbol_real_code_also_uses() {
    let before = graph_of(vec![
        ("src/a.ts", vec![], vec![]),
        ("src/b.ts", vec![], vec![]),
        ("src/a.test.ts", vec![], vec![]),
    ]);
    let helper = symbol("src/a.ts", "used", 1);
    let caller = symbol("src/b.ts", "caller", 1);
    let test = symbol("src/a.test.ts", "uses_helper", 1);
    let after = graph_of(vec![
        ("src/a.ts", vec![helper.clone()], vec![]),
        (
            "src/b.ts",
            vec![caller.clone()],
            vec![edge(&caller, &helper, RelationshipKind::Calls)],
        ),
        (
            "src/a.test.ts",
            vec![test.clone()],
            vec![edge(&test, &helper, RelationshipKind::Tests)],
        ),
    ]);
    let findings = run(&DeadOnArrival, &before, &after, FILES);

    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn negative_a_new_entry_point_nothing_calls_yet() {
    // A new CLI command or exported function has no inbound edges at all.
    // Requiring a test edge is what keeps this detector off ordinary new code.
    let before = graph_of(vec![("src/a.ts", vec![], vec![])]);
    let entry = symbol("src/a.ts", "newCommand", 1);
    let after = graph_of(vec![("src/a.ts", vec![entry], vec![])]);
    let findings = run(&DeadOnArrival, &before, &after, FILES);

    assert!(findings.is_empty(), "{findings:#?}");
}

// ── the set as a whole ───────────────────────────────────────

#[test]
fn every_shipped_detector_names_itself_and_what_it_looks_for() {
    for detector in detectors::all() {
        assert!(!detector.name().is_empty());
        assert!(
            detector.name().chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "{} is half of every id it produces, so it must be stable and kebab-case",
            detector.name()
        );
        assert!(!detector.describes().is_empty());
    }
}

#[test]
fn a_detector_held_back_records_why() {
    // An absent detector is indistinguishable from one that found nothing.
    let held = detectors::held_back();
    assert_eq!(held.len(), 2);
    for entry in held {
        assert!(!entry.reason.is_empty(), "{} was held back silently", entry.name);
    }
    let names: Vec<&str> = held.iter().map(|h| h.name).collect();
    assert!(names.contains(&"scope-creep"));
    assert!(names.contains(&"special-casing"));

    // `assertion-removal` was held back because assertion calls were not in the
    // graph. They are now, recorded during the same parse the symbols come
    // from, so it ships. A name may appear on one list or the other, never
    // both: that is the whole point of publishing what did not run.
    assert!(
        !names.contains(&"assertion-removal"),
        "assertion-removal ships now and must not also be listed as held back"
    );
    assert!(
        detectors::all().iter().any(|d| d.name() == "assertion-removal"),
        "assertion-removal was removed from the held back list without being shipped"
    );
}

#[test]
fn an_unchanged_repository_produces_no_findings_at_all() {
    // The property that decides whether anyone leaves this switched on.
    let graph = covered("fn target(a: i32)", true);
    for detector in detectors::all() {
        let findings = run(detector, &graph, &graph, FILES);
        assert!(
            findings.is_empty(),
            "{} fired on an unchanged repository: {findings:#?}",
            detector.name()
        );
    }
}
