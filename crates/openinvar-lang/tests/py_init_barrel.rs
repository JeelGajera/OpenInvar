// Exercises the python module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "python")]

#[path = "common/py.rs"]
mod common;

use common::*;
use openinvar_core::ir::RelationshipKind;

#[test]
fn importing_from_a_package_follows_its_init_re_exports() {
    let repo = analyze("language_features");
    let routes = file(&repo, "api/routes.py");

    // `from ..models import UserPayload` where `models/__init__.py` does
    // `from .user import UserPayload`. The definition is a module deeper.
    assert!(
        has_edge(
            routes,
            RelationshipKind::Imports,
            "models/user.py::UserPayload::class"
        ),
        "the re-export chain should lead to the definition, have: {:?}",
        targets(routes, RelationshipKind::Imports)
    );
}

#[test]
fn dunder_all_is_recorded_as_the_public_surface() {
    let repo = analyze("language_features");
    let init = file(&repo, "models/__init__.py");

    let exported: Vec<&str> = init
        .re_exports
        .iter()
        .map(|e| e.exported_name.as_str())
        .collect();

    assert!(exported.contains(&"UserPayload"), "got {exported:?}");
    assert!(exported.contains(&"Order"), "got {exported:?}");
}
