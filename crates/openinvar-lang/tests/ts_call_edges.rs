// Exercises the typescript module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "typescript")]

use std::path::{Path, PathBuf};

use openinvar_core::ir::{RelationshipKind, RepoIR};
use openinvar_core::scan::{walk_source_files_with_config, ScanConfig};
use openinvar_lang::lang::typescript::analyze_files;
use openinvar_lang::lang::typescript::language::is_supported_source_file;

fn analyze(root: &Path) -> RepoIR {
    let files =
        walk_source_files_with_config(root, &ScanConfig::default_enabled(), is_supported_source_file)
            .unwrap();
    analyze_files(root, &files).unwrap()
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapter-ts/call_edges")
        .canonicalize()
        .unwrap()
}

/// Every edge of `kind` originating in the consumer file, as (target, line).
fn edges_of(ir: &RepoIR, kind: RelationshipKind) -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = ir
        .files
        .iter()
        .filter(|f| f.file.ends_with("consumer.ts"))
        .flat_map(|f| f.relationships.iter())
        .filter(|r| r.kind == kind)
        .map(|r| (r.to.clone(), r.line))
        .collect();
    out.sort();
    out
}

#[test]
fn a_call_to_an_imported_function_resolves_to_its_definition() {
    let ir = analyze(&fixture());
    let calls = edges_of(&ir, RelationshipKind::Calls);

    assert_eq!(
        calls.len(),
        1,
        "expected exactly one call edge, got {calls:?}"
    );
    assert!(
        calls[0].0.contains("services.ts") && calls[0].0.contains("formatName"),
        "call should resolve to the definition of formatName, got {}",
        calls[0].0
    );
}

#[test]
fn a_call_through_a_renamed_import_resolves_to_the_canonical_symbol() {
    // The source calls `fmt(...)`, never `formatName(...)`. An edge naming the
    // local alias would be a different symbol than the one that runs, which is
    // the whole point of alias-aware resolution.
    let ir = analyze(&fixture());
    let calls = edges_of(&ir, RelationshipKind::Calls);

    assert!(
        calls.iter().all(|(to, _)| !to.ends_with("::fmt::function")),
        "call edge points at the local alias rather than the definition: {calls:?}"
    );
}

#[test]
fn new_records_an_instantiation_of_the_imported_class() {
    let ir = analyze(&fixture());
    let news = edges_of(&ir, RelationshipKind::Instantiates);

    assert_eq!(
        news.len(),
        1,
        "expected exactly one instantiation edge, got {news:?}"
    );
    assert!(
        news[0].0.contains("services.ts") && news[0].0.contains("UserService"),
        "instantiation should resolve to UserService, got {}",
        news[0].0
    );
}

#[test]
fn a_method_call_is_not_recorded_as_a_call_to_the_type() {
    // `service.handle()` is already recorded as a property access on the
    // receiver's declared type. Emitting Calls to the *type* as well would
    // claim the type was called, and would double every row for one location.
    let ir = analyze(&fixture());
    let calls = edges_of(&ir, RelationshipKind::Calls);

    assert!(
        calls
            .iter()
            .all(|(to, _)| !to.contains("UserService") || !to.contains("::class")),
        "a method call produced a call edge to the receiver's type: {calls:?}"
    );
}

#[test]
fn an_unresolvable_callee_records_nothing() {
    // `setTimeout` and `console.log` name nothing this file can resolve.
    // Recording them against a placeholder target would put edges in the graph
    // pointing at names rather than symbols, and blast-radius would count them
    // as dependents. This is the property that keeps call edges gate-safe.
    let ir = analyze(&fixture());

    for kind in [RelationshipKind::Calls, RelationshipKind::Instantiates] {
        for (to, line) in edges_of(&ir, kind.clone()) {
            assert!(
                !to.contains("unresolved"),
                "{kind:?} edge at line {line} kept an unresolved target: {to}"
            );
            assert!(
                !to.contains("setTimeout") && !to.contains("console"),
                "{kind:?} edge recorded for a name that resolves to nothing: {to}"
            );
        }
    }
}

#[test]
fn a_function_nothing_calls_gains_no_call_edge() {
    // Guards the inverse of the resolution tests: `unusedHelper` is exported
    // and never called, so a call edge naming it would mean the extractor is
    // matching by name rather than by binding.
    let ir = analyze(&fixture());

    let any_unused = ir
        .files
        .iter()
        .flat_map(|f| f.relationships.iter())
        .any(|r| r.kind == RelationshipKind::Calls && r.to.contains("unusedHelper"));

    assert!(!any_unused, "recorded a call to a function nobody calls");
}

#[test]
fn call_edges_are_deterministic() {
    // The collector sorts, because tree traversal order is not a contract.
    let first = edges_of(&analyze(&fixture()), RelationshipKind::Calls);
    let second = edges_of(&analyze(&fixture()), RelationshipKind::Calls);
    assert_eq!(first, second);
}
