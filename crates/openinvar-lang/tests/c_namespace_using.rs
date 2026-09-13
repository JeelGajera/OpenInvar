// Exercises the c module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "c")]

#[path = "common/c.rs"]
mod common;

use common::*;
use openinvar_core::ir::RelationshipKind;

#[test]
fn a_using_alias_resolves_to_the_aliased_class() {
    let repo = analyze("language_features");
    let render = file(&repo, "src/render.cpp");

    let alias = render
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::Imports && r.alias.as_deref() == Some("Figure"))
        .expect("`using Figure = geometry::Circle;` should be recorded");

    assert!(
        alias.to.ends_with("include/shapes.hpp::Circle::class"),
        "the namespace qualifier must not stop resolution, got {}",
        alias.to
    );
}

#[test]
fn members_reached_through_the_alias_are_attributed_to_the_real_class() {
    let repo = analyze("language_features");
    let render = file(&repo, "src/render.cpp");

    let props = properties_for(
        render,
        RelationshipKind::AccessesProperty,
        "include/shapes.hpp::Circle::class",
    );
    assert!(props.contains(&"radius".to_string()), "got {props:?}");
    assert!(
        props.contains(&"area".to_string()),
        "a method call through the alias is a member use too, got {props:?}"
    );
}
