// Exercises the c module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "c")]

#[path = "common/c.rs"]
mod common;

use common::*;
use openinvar_core::ir::RelationshipKind;

#[test]
fn arrow_access_is_attributed_to_the_declared_type() {
    let repo = analyze("language_features");
    let mapper = file(&repo, "src/mapper.c");

    let props = properties_for(
        mapper,
        RelationshipKind::AccessesProperty,
        "include/user_payload.h::UserPayload::class",
    );
    assert!(props.contains(&"email".to_string()), "got {props:?}");
    assert!(props.contains(&"user_id".to_string()), "got {props:?}");
}

#[test]
fn each_struct_receives_only_its_own_members() {
    let repo = analyze("language_features");
    let mapper = file(&repo, "src/mapper.c");

    let user_props = properties_for(
        mapper,
        RelationshipKind::AccessesProperty,
        "include/user_payload.h::UserPayload::class",
    );
    let order_props = properties_for(
        mapper,
        RelationshipKind::AccessesProperty,
        "include/user_payload.h::Order::class",
    );

    assert!(
        !user_props.contains(&"order_id".to_string()),
        "`order_id` belongs to Order; got {user_props:?}"
    );
    assert_eq!(order_props, vec!["order_id".to_string()]);
}

#[test]
fn a_type_reference_does_not_create_a_second_definition() {
    // Regression: `struct UserPayload *p` in a parameter list is also a
    // `struct_specifier`, and treating it as a definition minted one symbol per
    // file that merely mentioned the type. Every struct name then resolved
    // ambiguously and its blast radius was split across disconnected nodes.
    let repo = analyze("language_features");
    let definitions = definitions_named(&repo, "UserPayload");

    assert_eq!(
        definitions.len(),
        1,
        "UserPayload is defined once, in the header; got {definitions:?}"
    );
    assert!(definitions[0].starts_with("include/user_payload.h::"));
}
