// Exercises the rust module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "rust")]

#[path = "common/rust.rs"]
mod common;

use common::*;
use openinvar_core::ir::RelationshipKind;

#[test]
fn derived_traits_become_implementation_edges() {
    let repo = analyze("language_features");
    let user = file(&repo, "models/user.rs");

    // `#[derive(Serialize, Deserialize)]` generates trait impls that appear
    // nowhere in the source; without them a change to the struct's fields looks
    // unrelated to whatever consumes its serialised form.
    assert!(
        has_edge(user, RelationshipKind::Implements, "ext::serde::package"),
        "derived third-party traits should resolve to the crate providing them"
    );
}

#[test]
fn prelude_derives_are_not_recorded_as_dependencies() {
    let repo = analyze("language_features");
    let user = file(&repo, "models/user.rs");

    // `Debug` and `Clone` are on nearly every type and resolve to nothing in
    // the repository; recording them would add an edge per struct that tells
    // the reader nothing.
    let derive_targets: Vec<&str> = user
        .relationships
        .iter()
        .filter(|r| r.kind == RelationshipKind::Implements)
        .map(|r| r.context.as_str())
        .collect();

    assert!(
        !derive_targets.iter().any(|c| c.contains("Debug")),
        "std prelude derives should be filtered out, got: {derive_targets:?}"
    );
    assert!(
        !derive_targets.iter().any(|c| c.contains("Clone")),
        "std prelude derives should be filtered out, got: {derive_targets:?}"
    );
}
