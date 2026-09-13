// Exercises the c module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "c")]

#[path = "common/c.rs"]
mod common;

use common::*;
use openinvar_core::ir::RelationshipKind;

#[test]
fn a_typedef_alias_resolves_to_the_underlying_struct() {
    let repo = analyze("language_features");
    let mapper = file(&repo, "src/mapper.c");

    let alias = mapper
        .relationships
        .iter()
        .find(|r| {
            r.kind == RelationshipKind::Imports && r.alias.as_deref() == Some("ResponseModel")
        })
        .expect("`typedef struct UserPayload ResponseModel;` should be recorded");

    assert!(
        alias
            .to
            .ends_with("include/user_payload.h::UserPayload::class"),
        "got {}",
        alias.to
    );
}

#[test]
fn the_self_naming_typedef_idiom_creates_no_edge() {
    // `typedef struct UserPayload { ... } UserPayload;` in the header names the
    // struct after itself; that is not a relationship between two things.
    let repo = analyze("language_features");
    let header = file(&repo, "include/user_payload.h");

    let self_aliases: Vec<&str> = header
        .relationships
        .iter()
        .filter(|r| r.alias.as_deref() == Some("UserPayload"))
        .map(|r| r.to.as_str())
        .collect();

    assert!(self_aliases.is_empty(), "got {self_aliases:?}");
}
