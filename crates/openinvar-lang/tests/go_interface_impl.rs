// Exercises the go module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "go")]

#[path = "common/go.rs"]
mod common;

use common::*;
use openinvar_core::ir::{RelationshipKind, SymbolKind};

#[test]
fn a_type_satisfying_an_interface_gets_an_implements_edge() {
    let repo = analyze("language_features");
    let store = file(&repo, "store/store.go");

    // `MemoryStore` has both `Get` and `Close`, so it satisfies `Reader`
    // without saying so anywhere in the source.
    let edge = store
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::Implements)
        .expect("structural conformance should be detected");

    assert!(
        edge.from.ends_with("MemoryStore::class"),
        "got {}",
        edge.from
    );
    assert!(edge.to.ends_with("Reader::interface"), "got {}", edge.to);
}

#[test]
fn interface_methods_are_recorded_against_the_interface() {
    let repo = analyze("language_features");
    let store = file(&repo, "store/store.go");

    let ids: Vec<&str> = store
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Method)
        .map(|s| s.id.as_str())
        .collect();

    assert!(
        ids.iter().any(|id| id.ends_with("Reader::Get::method")),
        "interface methods define the required set, got {ids:?}"
    );
}

#[test]
fn a_type_missing_a_method_does_not_implement_the_interface() {
    let repo = analyze("language_features");

    // `UserPayload` has no methods at all and must not match `Reader`.
    let implements: Vec<String> = repo
        .files
        .iter()
        .flat_map(|f| f.relationships.iter())
        .filter(|r| r.kind == RelationshipKind::Implements)
        .map(|r| format!("{} -> {}", r.from, r.to))
        .collect();

    assert!(
        !implements.iter().any(|e| e.contains("UserPayload")),
        "a struct with no methods cannot satisfy a non-empty interface, got {implements:?}"
    );
}
