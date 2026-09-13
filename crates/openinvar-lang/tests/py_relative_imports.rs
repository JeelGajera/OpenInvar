// Exercises the python module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "python")]

#[path = "common/py.rs"]
mod common;

use common::*;
use openinvar_core::ir::RelationshipKind;

#[test]
fn a_relative_import_of_a_function_resolves_to_the_function() {
    // Regression: the index held only classes, so importing a function fell
    // through to a fabricated `ext::<root>::package` node. That did not just
    // fail to resolve — it asserted a third-party dependency that never existed.
    let repo = analyze("language_features");
    let routes = file(&repo, "api/routes.py");

    assert!(
        has_edge(
            routes,
            RelationshipKind::Imports,
            "models/user.py::normalize_email::function"
        ),
        "have: {:?}",
        targets(routes, RelationshipKind::Imports)
    );
}

#[test]
fn a_local_module_is_never_reported_as_an_external_package() {
    let repo = analyze("language_features");
    let routes = file(&repo, "api/routes.py");

    let external: Vec<String> = targets(routes, RelationshipKind::Imports)
        .into_iter()
        .filter(|t| t.starts_with("ext::"))
        .collect();

    assert!(
        !external.iter().any(|t| t.contains("models")),
        "`..models` is in this repository; got {external:?}"
    );
}

#[test]
fn class_and_function_imports_from_the_same_module_both_resolve() {
    let repo = analyze("language_features");
    let routes = file(&repo, "api/routes.py");

    assert!(
        has_edge(
            routes,
            RelationshipKind::Imports,
            "models/user.py::UserFilter::class"
        ),
        "have: {:?}",
        targets(routes, RelationshipKind::Imports)
    );
    assert!(has_edge(
        routes,
        RelationshipKind::Imports,
        "models/user.py::normalize_email::function"
    ));
}
