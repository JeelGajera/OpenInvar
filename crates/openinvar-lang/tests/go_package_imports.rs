// Exercises the go module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "go")]

#[path = "common/go.rs"]
mod common;

use common::*;
use openinvar_core::ir::RelationshipKind;

#[test]
fn a_package_import_points_at_the_package_not_a_member() {
    let repo = analyze("language_features");
    let handler = file(&repo, "handlers/user.go");

    // Go imports a package, never a symbol. Pointing the edge at a member
    // picked from the package made the target depend on filename sort order.
    let import = handler
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::Imports && r.alias.as_deref() == Some("m"))
        .expect("the aliased models import should be recorded");

    assert!(
        import.to.ends_with("models::models::module"),
        "expected the package node, got {}",
        import.to
    );
}

#[test]
fn adding_a_file_to_a_package_does_not_move_existing_import_edges() {
    // `models` holds both user.go and order.go. Whichever sorts first must not
    // change what an importer's edge points at.
    let repo = analyze("language_features");
    let handler = file(&repo, "handlers/user.go");

    let targets: Vec<&str> = handler
        .relationships
        .iter()
        .filter(|r| r.kind == RelationshipKind::Imports)
        .map(|r| r.to.as_str())
        .collect();

    assert!(
        !targets.iter().any(|t| t.ends_with("Order::class")),
        "an import of the package must not resolve to an arbitrary type in it, got {targets:?}"
    );
}

#[test]
fn third_party_imports_resolve_to_an_external_package() {
    let repo = analyze("language_features");
    let handler = file(&repo, "handlers/user.go");
    assert!(
        has_edge(handler, RelationshipKind::Imports, "ext::fmt::package"),
        "the standard library is external to the repository"
    );
}

#[test]
fn qualified_references_reach_the_specific_type() {
    let repo = analyze("language_features");
    let handler = file(&repo, "handlers/user.go");

    // The package edge says *that* we depend on `models`; the qualified
    // references say *which* types, which is what blast radius needs.
    assert!(
        has_edge(
            handler,
            RelationshipKind::UsesType,
            "models/user.go::UserPayload::class"
        ),
        "`*m.UserPayload` should produce an edge to the type"
    );
    assert!(
        has_edge(
            handler,
            RelationshipKind::UsesType,
            "models/order.go::Order::class"
        ),
        "`*m.Order` should produce an edge to the type"
    );
}
