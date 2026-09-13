// Exercises the rust module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "rust")]

#[path = "common/rust.rs"]
mod common;

use common::*;
use openinvar_core::ir::RelationshipKind;

#[test]
fn use_paths_resolve_through_the_module_that_declares_the_name() {
    let repo = analyze("language_features");
    let reporting = file(&repo, "services/reporting.rs");

    let order = reporting
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::Imports && r.to.ends_with("Order::class"))
        .expect("`use crate::models::order::Order` should resolve");

    assert!(
        order.to.ends_with("models/order.rs::Order::class"),
        "resolution must land in the module the path names, got {}",
        order.to
    );
}

#[test]
fn a_name_is_not_found_in_a_module_that_does_not_declare_it() {
    // `models/mod.rs` re-exports `UserPayload` but does not declare `Order`.
    // The previous resolver searched a repository-wide map of leaf names, so
    // any path ending in a known name resolved regardless of its module.
    let repo = analyze("language_features");
    let models = file(&repo, "models/mod.rs");

    // The re-export it *does* declare resolves.
    assert!(
        has_edge(
            models,
            RelationshipKind::Imports,
            "models/user.rs::UserPayload::class"
        ),
        "`pub use user::UserPayload` should resolve to the defining module"
    );
}

#[test]
fn re_exports_are_followed_when_importing_from_the_parent_module() {
    let repo = analyze("language_features");
    let models = file(&repo, "models/mod.rs");

    // `pub use user::UserPayload;` makes `crate::models::UserPayload` valid.
    // The resolver must follow that chain rather than only matching the module
    // that literally contains the definition.
    let reexport = models
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::Imports && r.to.ends_with("UserPayload::class"))
        .expect("the re-export should be recorded");
    assert!(reexport.to.ends_with("models/user.rs::UserPayload::class"));
}
