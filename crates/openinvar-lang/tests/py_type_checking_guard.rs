// Exercises the python module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "python")]

#[path = "common/py.rs"]
mod common;

use common::*;
use openinvar_core::ir::RelationshipKind;

#[test]
fn imports_inside_a_type_checking_guard_are_still_dependencies() {
    let repo = analyze("language_features");
    let routes = file(&repo, "api/routes.py");

    // An `if TYPE_CHECKING:` import does not execute at runtime, but it is a
    // genuine compile-time dependency: changing the imported symbol breaks the
    // annotations that reference it.
    assert!(
        has_edge(
            routes,
            RelationshipKind::Imports,
            "models/order.py::Order::class"
        ),
        "guarded imports must still be recorded, have: {:?}",
        targets(routes, RelationshipKind::Imports)
    );
}

#[test]
fn the_typing_module_itself_resolves_externally() {
    let repo = analyze("language_features");
    let routes = file(&repo, "api/routes.py");
    assert!(
        has_edge(routes, RelationshipKind::Imports, "ext::typing::package"),
        "have: {:?}",
        targets(routes, RelationshipKind::Imports)
    );
}
