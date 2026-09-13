// Exercises the go module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "go")]

#[path = "common/go.rs"]
mod common;

use common::*;
use openinvar_core::ir::RelationshipKind;

#[test]
fn field_access_is_attributed_to_the_parameter_type() {
    let repo = analyze("language_features");
    let handler = file(&repo, "handlers/user.go");

    let props = properties_for(
        handler,
        RelationshipKind::AccessesProperty,
        "models/user.go::UserPayload::class",
    );
    assert!(props.contains(&"Email".to_string()), "got {props:?}");
    assert!(props.contains(&"UserID".to_string()), "got {props:?}");
}

#[test]
fn each_type_receives_only_its_own_fields() {
    let repo = analyze("language_features");
    let handler = file(&repo, "handlers/user.go");

    let user_props = properties_for(
        handler,
        RelationshipKind::AccessesProperty,
        "models/user.go::UserPayload::class",
    );
    let order_props = properties_for(
        handler,
        RelationshipKind::AccessesProperty,
        "models/order.go::Order::class",
    );

    assert!(
        !user_props.contains(&"OrderID".to_string()),
        "UserPayload has no OrderID; got {user_props:?}"
    );
    assert_eq!(order_props, vec!["OrderID".to_string()]);
}

#[test]
fn package_qualifiers_are_not_treated_as_field_access() {
    // `fmt.Println` is a package selector, not a member read. The previous
    // implementation emitted a property-access edge for every selector in the
    // file, so the standard library appeared as an accessed object.
    let repo = analyze("language_features");
    let handler = file(&repo, "handlers/user.go");

    let accesses = edges(handler, RelationshipKind::AccessesProperty);
    assert!(
        !accesses.iter().any(|r| r.to.contains("fmt")),
        "package selectors must not become property accesses, got: {:?}",
        accesses.iter().map(|r| r.to.as_str()).collect::<Vec<_>>()
    );
}
