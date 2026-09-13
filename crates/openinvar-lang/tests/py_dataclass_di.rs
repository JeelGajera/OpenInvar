// Exercises the python module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "python")]

#[path = "common/py.rs"]
mod common;

use common::*;
use openinvar_core::ir::SymbolKind;

#[test]
fn dataclass_fields_are_extracted() {
    let repo = analyze("language_features");
    let user = file(&repo, "models/user.py");

    // `@dataclass` generates `__init__` from the annotations, so the fields
    // are the constructor's signature.
    let filter_fields: Vec<&str> = user
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Property && s.id.contains("UserFilter"))
        .map(|s| s.name.as_str())
        .collect();

    assert!(filter_fields.contains(&"term"), "got {filter_fields:?}");
    assert!(filter_fields.contains(&"limit"), "got {filter_fields:?}");
}

#[test]
fn a_decorated_class_is_still_a_class() {
    let repo = analyze("language_features");
    let user = file(&repo, "models/user.py");
    assert_eq!(symbol_kind(user, "UserFilter"), SymbolKind::Class);
}
