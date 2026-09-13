// Exercises the python module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "python")]

#[path = "common/py.rs"]
mod common;

use common::*;
use openinvar_core::ir::RelationshipKind;

#[test]
fn a_star_import_resolves_to_the_module_it_names() {
    let repo = analyze("language_features");
    let star = file(&repo, "legacy/star.py");

    // `from ..models.user import *` brings in everything, so the dependency is
    // on the module rather than any single name.
    assert!(
        has_edge(
            star,
            RelationshipKind::Imports,
            "models/user.py::module::module"
        ),
        "have: {:?}",
        targets(star, RelationshipKind::Imports)
    );
}

#[test]
fn a_local_star_import_is_not_reported_as_external() {
    let repo = analyze("language_features");
    let star = file(&repo, "legacy/star.py");

    let external: Vec<String> = targets(star, RelationshipKind::Imports)
        .into_iter()
        .filter(|t| t.starts_with("ext::"))
        .collect();
    assert!(external.is_empty(), "got {external:?}");
}
