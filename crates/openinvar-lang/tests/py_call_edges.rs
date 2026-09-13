// Exercises the python module, so it compiles only when that language is
// enabled. A slim build otherwise tries to compile a test for a module it
// does not carry.
#![cfg(feature = "python")]

use std::path::{Path, PathBuf};

use openinvar_core::ir::{RelationshipKind, RepoIR};
use openinvar_lang::lang::python::analyze_files;

fn all_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root).into_iter().flatten() {
        if entry.path().is_file()
            && matches!(
                entry.path().extension().and_then(|x| x.to_str()),
                Some("py" | "pyi")
            )
        {
            out.push(entry.path().to_path_buf());
        }
    }
    out
}

fn analyze(root: &Path) -> RepoIR {
    analyze_files(root, &all_files(root)).expect("analysis must succeed")
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/adapter-py/call_edges")
}

/// Every edge of `kind` originating in the consumer file, as (target, line).
fn edges_of(ir: &RepoIR, kind: RelationshipKind) -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = ir
        .files
        .iter()
        .filter(|f| f.file.ends_with("consumer.py"))
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
        calls[0].0.contains("services.py") && calls[0].0.contains("format_name"),
        "call should resolve to the definition of format_name, got {}",
        calls[0].0
    );
}

#[test]
fn a_call_through_a_renamed_import_resolves_to_the_canonical_symbol() {
    // The source calls `fmt(...)`, never `format_name(...)`. An edge naming the
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
fn calling_a_class_records_an_instantiation() {
    // The load-bearing Python-specific case. `UserService()` and `fmt()` are
    // the same node kind, so the split between Calls and Instantiates is made
    // from the resolved target's kind. Getting it from the spelling of the
    // name instead would be a naming convention dressed up as an analysis.
    let ir = analyze(&fixture());
    let news = edges_of(&ir, RelationshipKind::Instantiates);

    assert_eq!(
        news.len(),
        1,
        "expected exactly one instantiation edge, got {news:?}"
    );
    assert!(
        news[0].0.contains("services.py") && news[0].0.contains("UserService"),
        "instantiation should resolve to UserService, got {}",
        news[0].0
    );
    assert!(
        news[0].0.ends_with("::class"),
        "an instantiation must target a class, got {}",
        news[0].0
    );
}

#[test]
fn an_attribute_call_is_not_recorded_as_a_call_to_the_receiver() {
    // `service.handle()` is already a property access on UserService. A Calls
    // edge to the class as well would claim the class was called, and would
    // double every row for one source location.
    let ir = analyze(&fixture());

    let all: Vec<(RelationshipKind, String)> = ir
        .files
        .iter()
        .filter(|f| f.file.ends_with("consumer.py"))
        .flat_map(|f| f.relationships.iter())
        .filter(|r| r.kind == RelationshipKind::Calls || r.kind == RelationshipKind::Instantiates)
        .map(|r| (r.kind.clone(), r.to.clone()))
        .collect();

    assert!(
        all.iter().all(|(_, to)| !to.contains("handle")),
        "an attribute call produced a call edge: {all:?}"
    );
    assert_eq!(
        all.len(),
        2,
        "expected exactly the one call and one instantiation, got {all:?}"
    );
}

#[test]
fn a_builtin_call_records_no_edge_and_no_diagnostic() {
    // `print(name)` and `len(name)` bind to no symbol in the repository. An
    // edge invented for them would be counted as a dependent by blast-radius,
    // and a diagnostic would fire in almost every Python file ever written —
    // burying the resolution warnings that a user can actually act on.
    let ir = analyze(&fixture());

    let consumer = ir
        .files
        .iter()
        .find(|f| f.file.ends_with("consumer.py"))
        .expect("consumer file in the analysis");

    let invented: Vec<&str> = consumer
        .relationships
        .iter()
        .filter(|r| r.kind == RelationshipKind::Calls)
        .map(|r| r.to.as_str())
        .filter(|to| to.contains("print") || to.contains("len"))
        .collect();

    assert!(
        invented.is_empty(),
        "a builtin call produced an edge: {invented:?}"
    );
    assert!(
        consumer.diagnostics.is_empty(),
        "unresolvable calls must not raise diagnostics, got {:?}",
        consumer.diagnostics
    );
}

#[test]
fn a_call_into_a_third_party_package_records_no_call_edge() {
    // `get("/health")` after `from requests import get` really does call
    // something — but the only id available is the package, and an edge saying
    // a *package* was called would be counted as a dependent of a symbol
    // nobody can open. The Imports edge already records the dependency, and
    // this matches what TypeScript does with the same shape of import.
    let ir = analyze(&fixture());

    let consumer = ir
        .files
        .iter()
        .find(|f| f.file.ends_with("consumer.py"))
        .expect("consumer file in the analysis");

    let external_calls: Vec<&str> = consumer
        .relationships
        .iter()
        .filter(|r| r.kind == RelationshipKind::Calls || r.kind == RelationshipKind::Instantiates)
        .map(|r| r.to.as_str())
        .filter(|to| to.starts_with("ext::"))
        .collect();

    assert!(
        external_calls.is_empty(),
        "recorded a call against a package rather than a symbol: {external_calls:?}"
    );
    assert!(
        consumer
            .relationships
            .iter()
            .any(|r| r.kind == RelationshipKind::Imports && r.to == "ext::requests::package"),
        "the import edge that carries the real dependency is missing"
    );
}

#[test]
fn call_edges_reach_a_user_stamped_resolved() {
    // Goes through `dispatch`, not `python::analyze_files`, because dispatch is
    // where the Tier 1 stamp is applied and therefore the only layer at which
    // this property is real. Asserting it against the adapter output would test
    // the default value instead, and pass for the wrong reason.
    //
    // It matters because the stamp is what lets `blast-radius` say "safe to
    // modify": a new edge kind that missed it would silently turn every Python
    // repository into one with structural regions.
    use openinvar_core::ir::Resolution;

    let root = fixture();
    let ir = openinvar_lang::dispatch::analyze_files(&root, &all_files(&root))
        .expect("dispatch analysis must succeed");

    let calls: Vec<(&str, u32, &Resolution)> = ir
        .files
        .iter()
        .flat_map(|f| f.relationships.iter())
        .filter(|r| r.kind == RelationshipKind::Calls || r.kind == RelationshipKind::Instantiates)
        .map(|r| (r.file.as_str(), r.line, &r.resolution))
        .collect();

    assert_eq!(
        calls.len(),
        2,
        "expected the call and the instantiation to survive dispatch, got {calls:?}"
    );
    assert!(
        calls.iter().all(|(_, _, res)| **res == Resolution::Resolved),
        "call edges left at the structural default: {calls:?}"
    );
}
