// Exercises the python module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "python")]

#[path = "common/py.rs"]
mod common;

use common::*;
use openinvar_core::ir::{RelationshipKind, SymbolKind};

#[test]
fn a_django_model_is_extracted_as_a_class() {
    let repo = analyze("language_features");
    let order = file(&repo, "models/order.py");
    assert_eq!(symbol_kind(order, "Order"), SymbolKind::Class);
}

#[test]
fn the_dotted_django_base_class_is_recorded() {
    let repo = analyze("language_features");
    let order = file(&repo, "models/order.py");

    // `class Order(models.Model)` — the dotted spelling must still resolve to
    // the package it comes from.
    assert!(
        has_edge(order, RelationshipKind::Extends, "ext::django::package"),
        "have: {:?}",
        targets(order, RelationshipKind::Extends)
    );
}
