// Exercises the go module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "go")]

#[path = "common/go.rs"]
mod common;

use common::*;
use openinvar_core::ir::RelationshipKind;

#[test]
fn a_constructor_taking_an_interface_records_the_dependency() {
    let repo = analyze("language_features");
    let handler = file(&repo, "handlers/user.go");

    // `func NewUserHandler(reader store.Reader) *UserHandler` — the handler
    // depends on the interface, which is the whole point of injecting it.
    assert!(
        has_edge(
            handler,
            RelationshipKind::UsesType,
            "store/store.go::Reader::interface"
        ),
        "constructor-injected interfaces are dependencies, have: {:?}",
        handler
            .relationships
            .iter()
            .map(|r| format!("{:?} -> {}", r.kind, r.to))
            .collect::<Vec<_>>()
    );
}

#[test]
fn struct_fields_holding_an_interface_are_recorded() {
    let repo = analyze("language_features");
    let handler = file(&repo, "handlers/user.go");

    // `type UserHandler struct { reader store.Reader }`
    assert!(
        has_edge(handler, RelationshipKind::UsesType, "Reader::interface"),
        "an interface-typed field is a dependency of the struct"
    );
}
