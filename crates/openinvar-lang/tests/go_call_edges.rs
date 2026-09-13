// Exercises the go module, so it compiles only when that language is
// enabled. A slim build otherwise tries to compile a test for a module it
// does not carry.
#![cfg(feature = "go")]

use std::path::{Path, PathBuf};

use openinvar_core::ir::{RelationshipKind, RepoIR};
use openinvar_lang::lang::go::analyze_files;

fn all_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root).into_iter().flatten() {
        if entry.path().is_file() && entry.path().extension().and_then(|x| x.to_str()) == Some("go")
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
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/adapter-go/call_edges")
}

/// Every call-like edge in the caller file, as (kind, target, line).
fn call_edges(ir: &RepoIR) -> Vec<(RelationshipKind, String, u32)> {
    let mut out: Vec<(RelationshipKind, String, u32)> = ir
        .files
        .iter()
        .filter(|f| f.file.ends_with("run.go"))
        .flat_map(|f| f.relationships.iter())
        .filter(|r| r.kind == RelationshipKind::Calls || r.kind == RelationshipKind::Instantiates)
        .map(|r| (r.kind.clone(), r.to.clone(), r.line))
        .collect();
    out.sort_by_key(|(_, to, line)| (*line, to.clone()));
    out
}

#[test]
fn a_cross_package_call_resolves_to_the_function_it_names() {
    // The case that decides whether Go call edges are worth having. A call
    // into another package is *always* written through the package name, so a
    // rule that skipped selectors — the rule TypeScript, Python and Rust use
    // for `obj.method()` — would leave Go with call edges that never cross a
    // file boundary.
    let ir = analyze(&fixture());
    let edges = call_edges(&ir);

    let cross = edges
        .iter()
        .find(|(_, to, _)| to.contains("NewUser"))
        .unwrap_or_else(|| panic!("no edge for models.NewUser, got {edges:?}"));

    assert_eq!(cross.0, RelationshipKind::Calls, "{cross:?}");
    assert!(
        cross.1.ends_with("models/user.go::NewUser::function"),
        "the call should resolve into the models package, got {}",
        cross.1
    );
}

#[test]
fn a_same_package_call_resolves_too() {
    let ir = analyze(&fixture());
    let edges = call_edges(&ir);

    assert!(
        edges.iter().any(|(kind, to, _)| *kind == RelationshipKind::Calls
            && to.ends_with("::describe::function")),
        "the same-package call to describe is missing: {edges:?}"
    );
}

#[test]
fn a_composite_literal_records_an_instantiation() {
    // `models.User{..}` is Go's construction syntax, so unlike Python's `Foo()`
    // it needs no inference from the target's kind — the syntax says it.
    let ir = analyze(&fixture());
    let edges = call_edges(&ir);

    let literal = edges
        .iter()
        .find(|(kind, to, _)| {
            *kind == RelationshipKind::Instantiates && to.ends_with("::User::class")
        })
        .unwrap_or_else(|| panic!("no instantiation of models.User, got {edges:?}"));

    assert!(literal.1.contains("models/user.go"), "{}", literal.1);
}

#[test]
fn a_type_conversion_is_not_recorded_as_a_call() {
    // `models.UserID(42)` is spelled exactly like a call and calls nothing.
    // Go has no syntax that distinguishes the two, so the resolved target's
    // kind is the only thing that can — the same rule Python needs for
    // `Foo()`. Left as `Calls`, this would appear under `--kind calls` as a
    // caller that does not exist.
    let ir = analyze(&fixture());
    let edges = call_edges(&ir);

    let conversion = edges
        .iter()
        .find(|(_, to, _)| to.contains("UserID"))
        .unwrap_or_else(|| panic!("no edge for models.UserID(42), got {edges:?}"));

    assert_eq!(
        conversion.0,
        RelationshipKind::Instantiates,
        "a conversion must not be a call, got {conversion:?}"
    );
}

#[test]
fn a_method_call_on_a_value_is_not_recorded_as_a_call() {
    // `user.Greeting()` selects on a value, not on a package. An edge here
    // would claim the receiver was called; where the receiver's type is
    // declared, the access is already recorded against that type instead.
    let ir = analyze(&fixture());
    let edges = call_edges(&ir);

    assert!(
        edges.iter().all(|(_, to, _)| !to.contains("Greeting")),
        "a method call produced a call edge: {edges:?}"
    );
}

#[test]
fn a_declared_receiver_still_attributes_the_method_to_its_type() {
    // The other half of the rule above: skipping the call edge is only honest
    // because the method still reaches the graph as a property access on the
    // receiver's declared type. This asserts the edge that carries it, so the
    // justification cannot quietly stop being true.
    let ir = analyze(&fixture());

    let attributed = ir
        .files
        .iter()
        .filter(|f| f.file.ends_with("run.go"))
        .flat_map(|f| f.relationships.iter())
        .filter(|r| r.kind == RelationshipKind::AccessesProperty)
        .any(|r| {
            r.to.ends_with("::User::class") && r.properties_accessed.iter().any(|p| p == "Greeting")
        });

    assert!(
        attributed,
        "a method called on a declared-type receiver was not attributed to that type"
    );
}

#[test]
fn builtins_and_third_party_calls_record_nothing() {
    // `len(greeting)` is a builtin and `fmt.Println` is the standard library.
    // Neither names a symbol in this graph, and no diagnostic is raised
    // because there is nothing here a user could fix.
    let ir = analyze(&fixture());
    let edges = call_edges(&ir);

    assert!(
        edges
            .iter()
            .all(|(_, to, _)| !to.contains("len") && !to.contains("Println")),
        "a builtin or third-party call produced an edge: {edges:?}"
    );

    let caller = ir
        .files
        .iter()
        .find(|f| f.file.ends_with("run.go"))
        .expect("run.go in the analysis");
    assert!(
        caller.diagnostics.is_empty(),
        "unresolvable calls must not raise diagnostics, got {:?}",
        caller.diagnostics
    );
}

#[test]
fn calls_only_ever_target_something_callable() {
    // The invariant behind the conversion rule, stated once and asserted
    // rather than left as a property of the fixture.
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
    // Through `dispatch`, where the Tier 1 stamp is applied — the only layer
    // at which this property is real. A new edge kind that missed it would
    // silently turn every Go repository into one with structural regions and
    // suppress the safety verdict.
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

    assert!(
        !calls.is_empty(),
        "no call-like edge survived dispatch at all"
    );
    assert!(
        calls.iter().all(|(_, _, res)| **res == Resolution::Resolved),
        "call edges left at the structural default: {calls:?}"
    );
}
