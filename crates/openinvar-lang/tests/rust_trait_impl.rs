// Exercises the rust module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "rust")]

#[path = "common/rust.rs"]
mod common;

use common::*;
use openinvar_core::ir::{RelationshipKind, SymbolKind};

#[test]
fn impl_trait_for_type_produces_an_implements_edge() {
    let repo = analyze("language_features");
    let user = file(&repo, "models/user.rs");

    let edge = user
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::Implements && r.to.ends_with("Identify::interface"))
        .expect("impl Identify for UserPayload should be recorded");

    assert!(
        edge.from.ends_with("UserPayload::class"),
        "the implementing type is the source of the edge, got {}",
        edge.from
    );
}

#[test]
fn traits_are_extracted_as_interfaces() {
    let repo = analyze("language_features");
    let user = file(&repo, "models/user.rs");
    assert_eq!(symbol_names(user, SymbolKind::Interface), vec!["Identify"]);
}

#[test]
fn impl_methods_are_qualified_by_their_owning_type() {
    let repo = analyze("language_features");
    let reporting = file(&repo, "services/reporting.rs");

    let methods: Vec<&str> = reporting
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Method)
        .map(|s| s.id.as_str())
        .collect();

    // Qualifying the id keeps two same-named methods on different types from
    // collapsing onto one graph node.
    assert!(
        methods
            .iter()
            .any(|id| id.ends_with("Reporter::summarize::method")),
        "method ids should carry their impl type, got {methods:?}"
    );
}
