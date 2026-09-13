// Exercises the rust module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "rust")]

#[path = "common/rust.rs"]
mod common;

use common::*;
use openinvar_core::ir::RelationshipKind;

#[test]
fn field_access_is_attributed_to_the_parameter_type() {
    let repo = analyze("language_features");
    let reporting = file(&repo, "services/reporting.rs");

    let props = properties_for(
        reporting,
        RelationshipKind::AccessesProperty,
        "models/user.rs::UserPayload::class",
    );
    assert!(props.contains(&"email".to_string()), "got {props:?}");
    assert!(props.contains(&"user_id".to_string()), "got {props:?}");
}

#[test]
fn each_type_receives_only_its_own_fields() {
    // Regression: property sets used to be merged across the whole file, so a
    // struct was reported as having fields belonging to a different struct.
    let repo = analyze("language_features");
    let reporting = file(&repo, "services/reporting.rs");

    let user_props = properties_for(
        reporting,
        RelationshipKind::AccessesProperty,
        "models/user.rs::UserPayload::class",
    );
    let order_props = properties_for(
        reporting,
        RelationshipKind::AccessesProperty,
        "models/order.rs::Order::class",
    );

    assert!(
        !user_props.contains(&"total".to_string()),
        "UserPayload has no `total` field; got {user_props:?}"
    );
    assert_eq!(
        order_props,
        vec!["total".to_string()],
        "Order is only accessed for `total`"
    );
}

#[test]
fn self_field_access_is_attributed_to_the_impl_type() {
    let repo = analyze("language_features");
    let user = file(&repo, "models/user.rs");

    // `impl Identify for UserPayload { fn identity(&self) { self.user_id } }`
    let props = properties_for(
        user,
        RelationshipKind::AccessesProperty,
        "UserPayload::class",
    );
    assert!(
        props.contains(&"user_id".to_string()),
        "`self.user_id` belongs to the impl's type, got {props:?}"
    );
}
