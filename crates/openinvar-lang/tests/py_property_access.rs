// Exercises the python module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "python")]

#[path = "common/py.rs"]
mod common;

use common::*;
use openinvar_core::ir::RelationshipKind;

#[test]
fn attribute_access_is_attributed_to_the_annotated_type() {
    let repo = analyze("language_features");
    let routes = file(&repo, "api/routes.py");

    let props = properties_for(
        routes,
        RelationshipKind::AccessesProperty,
        "models/user.py::UserPayload::class",
    );
    assert!(props.contains(&"email".to_string()), "got {props:?}");
    assert!(props.contains(&"user_id".to_string()), "got {props:?}");
}

#[test]
fn each_annotated_type_receives_only_its_own_attributes() {
    let repo = analyze("language_features");
    let routes = file(&repo, "api/routes.py");

    let user_props = properties_for(
        routes,
        RelationshipKind::AccessesProperty,
        "models/user.py::UserPayload::class",
    );
    let filter_props = properties_for(
        routes,
        RelationshipKind::AccessesProperty,
        "models/user.py::UserFilter::class",
    );

    assert!(
        !user_props.contains(&"term".to_string()),
        "`term` belongs to UserFilter; got {user_props:?}"
    );
    assert_eq!(filter_props, vec!["term".to_string()]);
}
