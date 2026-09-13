// Exercises the python module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "python")]

#[path = "common/py.rs"]
mod common;

use common::*;
use openinvar_core::ir::RelationshipKind;

#[test]
fn dependency_injection_helpers_are_imported_from_the_framework() {
    let repo = analyze("language_features");
    let routes = file(&repo, "api/routes.py");
    assert!(
        has_edge(routes, RelationshipKind::Imports, "ext::fastapi::package"),
        "have: {:?}",
        targets(routes, RelationshipKind::Imports)
    );
}

#[test]
fn a_function_used_as_a_dependency_resolves_to_its_definition() {
    let repo = analyze("language_features");
    let routes = file(&repo, "api/routes.py");

    // `Depends(normalize_email)` only works because `normalize_email` was
    // imported; that import is a real edge to the defining module.
    assert!(
        has_edge(
            routes,
            RelationshipKind::Imports,
            "models/user.py::normalize_email::function"
        ),
        "an imported function must resolve to its definition, have: {:?}",
        targets(routes, RelationshipKind::Imports)
    );
}
