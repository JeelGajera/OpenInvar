// Exercises the rust module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "rust")]

#[path = "common/rust.rs"]
mod common;

use common::*;
use openinvar_core::ir::RelationshipKind;

#[test]
fn braced_use_groups_expand_to_one_edge_per_name() {
    let repo = analyze("language_features");
    let reporting = file(&repo, "services/reporting.rs");

    // `use crate::models::user::{Identify, UserPayload as Customer};`
    assert!(
        has_edge(reporting, RelationshipKind::Imports, "Identify::interface"),
        "each name in a use group needs its own edge"
    );
    assert!(
        has_edge(reporting, RelationshipKind::Imports, "UserPayload::class"),
        "each name in a use group needs its own edge"
    );
}

#[test]
fn aliases_inside_a_use_group_are_preserved() {
    let repo = analyze("language_features");
    let reporting = file(&repo, "services/reporting.rs");

    let aliased = reporting
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::Imports && r.alias.as_deref() == Some("Customer"))
        .expect("`UserPayload as Customer` should keep its alias");

    assert!(
        aliased.to.ends_with("models/user.rs::UserPayload::class"),
        "the alias must resolve to the real symbol, got {}",
        aliased.to
    );
}
