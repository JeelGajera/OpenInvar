// Exercises the rust module, so it compiles only when that language is
// enabled. A slim build otherwise tries to compile a test for a module it
// does not carry.
#![cfg(feature = "rust")]

use std::path::{Path, PathBuf};

use openinvar_core::ir::{RelationshipKind, RepoIR};
use openinvar_lang::lang::rust::analyze_files;

fn all_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root).into_iter().flatten() {
        if entry.path().is_file() && entry.path().extension().and_then(|x| x.to_str()) == Some("rs")
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
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/adapter-rust/call_edges")
}

/// Every call-like edge in the consumer file, as (kind, target, line).
fn call_edges(ir: &RepoIR) -> Vec<(RelationshipKind, String, u32)> {
    let mut out: Vec<(RelationshipKind, String, u32)> = ir
        .files
        .iter()
        .filter(|f| f.file.ends_with("consumer.rs"))
        .flat_map(|f| f.relationships.iter())
        .filter(|r| r.kind == RelationshipKind::Calls || r.kind == RelationshipKind::Instantiates)
        .map(|r| (r.kind.clone(), r.to.clone(), r.line))
        .collect();
    out.sort_by_key(|(_, to, line)| (*line, to.clone()));
    out
}

#[test]
fn an_associated_function_call_names_the_method_not_the_type() {
    // `UserService::new(..)` runs the method. Recording it as an
    // instantiation of UserService would be reading meaning into the name
    // `new`, which returns `Self` only by convention — and it would make
    // `blast-radius UserService::new` blind to its own callers.
    let ir = analyze(&fixture());
    let edges = call_edges(&ir);

    let associated = edges
        .iter()
        .find(|(_, to, _)| to.contains("::new::"))
        .unwrap_or_else(|| panic!("no edge for UserService::new, got {edges:?}"));

    assert_eq!(
        associated.0,
        RelationshipKind::Calls,
        "an associated function call must be a call, got {associated:?}"
    );
    assert!(
        associated.1.ends_with("::UserService::new::method"),
        "the edge should name the method symbol, got {}",
        associated.1
    );
}

#[test]
fn a_struct_literal_records_an_instantiation() {
    // `UserService { .. }` is Rust's actual construction syntax, and the one
    // shape that is an instantiation outright rather than by inference.
    let ir = analyze(&fixture());
    let edges = call_edges(&ir);

    let literal = edges
        .iter()
        .find(|(kind, to, _)| {
            *kind == RelationshipKind::Instantiates && to.ends_with("::UserService::class")
        })
        .unwrap_or_else(|| panic!("no instantiation of UserService, got {edges:?}"));

    assert!(
        literal.1.ends_with("::class"),
        "an instantiation must target the type, got {}",
        literal.1
    );
}

#[test]
fn a_tuple_struct_constructor_is_promoted_to_an_instantiation() {
    // `Wrapper(1)` is spelled exactly like a function call. The resolved
    // target's kind is what makes it construction — the same rule Python
    // needs for every `Foo()`, and the reason the kind is decided after
    // resolution rather than at the call site.
    let ir = analyze(&fixture());
    let edges = call_edges(&ir);

    let wrapper = edges
        .iter()
        .find(|(_, to, _)| to.contains("Wrapper"))
        .unwrap_or_else(|| panic!("no edge for Wrapper(1), got {edges:?}"));

    assert_eq!(
        wrapper.0,
        RelationshipKind::Instantiates,
        "a tuple-struct constructor should resolve to an instantiation, got {wrapper:?}"
    );
}

#[test]
fn a_call_through_a_renamed_import_resolves_to_the_canonical_symbol() {
    // The source calls `fmt(..)`, never `format_name(..)`. An edge naming the
    // local alias would be a different symbol than the one that runs.
    let ir = analyze(&fixture());
    let edges = call_edges(&ir);

    assert!(
        edges
            .iter()
            .any(|(kind, to, _)| *kind == RelationshipKind::Calls
                && to.ends_with("::format_name::function")),
        "the renamed call did not resolve to format_name: {edges:?}"
    );
    assert!(
        edges.iter().all(|(_, to, _)| !to.contains("::fmt::")),
        "call edge points at the local alias rather than the definition: {edges:?}"
    );
}

#[test]
fn a_method_call_is_not_recorded_as_a_call_to_the_receiver() {
    // `service.handle()` is already a property access on UserService. A call
    // edge to the type as well would claim the type was called, and would
    // double every row for one source location.
    let ir = analyze(&fixture());
    let edges = call_edges(&ir);

    assert!(
        edges.iter().all(|(_, to, _)| !to.contains("handle")),
        "a method call produced a call edge: {edges:?}"
    );
}

#[test]
fn prelude_names_and_macros_record_nothing() {
    // `drop(direct)` names a prelude function and `println!` is a macro
    // invocation, not a call node. Neither names a symbol in the graph, and
    // an invented edge would be counted by blast-radius as a dependent.
    //
    // No diagnostic either: there is nothing here a user could fix, and a
    // warning per `println!` would bury the resolution warnings that matter.
    let ir = analyze(&fixture());
    let edges = call_edges(&ir);

    assert!(
        edges
            .iter()
            .all(|(_, to, _)| !to.contains("drop") && !to.contains("println")),
        "a prelude name or macro produced an edge: {edges:?}"
    );

    let consumer = ir
        .files
        .iter()
        .find(|f| f.file.ends_with("consumer.rs"))
        .expect("consumer file in the analysis");
    assert!(
        consumer.diagnostics.is_empty(),
        "unresolvable calls must not raise diagnostics, got {:?}",
        consumer.diagnostics
    );
}

#[test]
fn a_fully_qualified_path_without_a_use_records_no_edge() {
    // `crate::services::unused_helper()` written inline. Resolving it would
    // mean guessing which `services` was meant; matching the leaf name across
    // the repository is the bug 0.2.0 fixed in this adapter.
    //
    // The README states this limit. The test exists so the limit cannot widen
    // by accident and leave the documentation quietly wrong.
    let ir = analyze(&fixture());
    let edges = call_edges(&ir);

    assert!(
        edges.iter().all(|(_, to, _)| !to.contains("unused_helper")),
        "a fully-qualified inline path resolved, widening a documented limit: {edges:?}"
    );
    assert_eq!(
        edges.len(),
        5,
        "expected exactly the five resolvable call-like edges, got {edges:?}"
    );
}

#[test]
fn a_tuple_enum_variant_is_construction_not_a_call() {
    // `Outcome::Ready("ok")` reads as a call and is not one — nothing ever
    // calls a variant. Left as `Calls`, it would put an edge in the graph
    // claiming a variant runs, and would show up under `--kind calls` as a
    // caller that does not exist.
    let ir = analyze(&fixture());
    let edges = call_edges(&ir);

    let variant = edges
        .iter()
        .find(|(_, to, _)| to.contains("Outcome::Ready"))
        .unwrap_or_else(|| panic!("no edge for Outcome::Ready, got {edges:?}"));

    assert_eq!(
        variant.0,
        RelationshipKind::Instantiates,
        "a tuple variant constructor must be an instantiation, got {variant:?}"
    );
}

#[test]
fn calls_only_ever_target_something_callable() {
    // The invariant behind the two promotions above, stated once. A `Calls`
    // edge that lands on a class or a variant is a claim that a type runs,
    // which is the kind of quiet wrongness `--kind calls` exists to avoid.
    let ir = analyze(&fixture());

    let miscast: Vec<(RelationshipKind, String, u32)> = call_edges(&ir)
        .into_iter()
        .filter(|(kind, to, _)| {
            *kind == RelationshipKind::Calls
                && !(to.ends_with("::function") || to.ends_with("::method"))
        })
        .collect();

    assert!(
        miscast.is_empty(),
        "call edges pointing at something that cannot be called: {miscast:?}"
    );
}

#[test]
fn call_edges_reach_a_user_stamped_resolved() {
    // Goes through `dispatch`, not `rust::analyze_files`, because dispatch is
    // where the Tier 1 stamp is applied and therefore the only layer at which
    // this property is real. A new edge kind that missed the stamp would
    // silently turn every Rust repository into one with structural regions
    // and suppress the safety verdict.
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
        5,
        "expected the five call-like edges to survive dispatch, got {calls:?}"
    );
    assert!(
        calls.iter().all(|(_, _, res)| **res == Resolution::Resolved),
        "call edges left at the structural default: {calls:?}"
    );
}
