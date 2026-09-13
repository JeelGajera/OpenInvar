// Exercises the python module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "python")]

#[path = "common/py.rs"]
mod common;

use common::*;
use openinvar_core::ir::{RelationshipKind, SymbolKind};

#[test]
fn pydantic_model_fields_are_extracted_as_properties() {
    let repo = analyze("language_features");
    let user = file(&repo, "models/user.py");

    let fields: Vec<&str> = user
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Property)
        .map(|s| s.name.as_str())
        .collect();

    // A model's annotated fields are its public contract even though nothing
    // assigns them — renaming one breaks every consumer of the serialised form.
    assert!(fields.contains(&"user_id"), "got {fields:?}");
    assert!(fields.contains(&"email"), "got {fields:?}");
    assert!(fields.contains(&"timestamp"), "got {fields:?}");
}

#[test]
fn the_base_model_is_recorded_as_a_dependency() {
    let repo = analyze("language_features");
    let user = file(&repo, "models/user.py");
    assert!(
        has_edge(user, RelationshipKind::Extends, "ext::pydantic::package"),
        "inheriting from BaseModel is a dependency on pydantic"
    );
}
