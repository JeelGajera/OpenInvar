// Exercises the c module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "c")]

#[path = "common/c.rs"]
mod common;

use common::*;
use openinvar_core::ir::{RelationshipKind, SymbolKind};

#[test]
fn a_function_taking_a_struct_pointer_records_the_dependency() {
    let repo = analyze("language_features");
    let mapper = file(&repo, "src/mapper.c");

    // C's equivalent of injection: the type arrives through the signature.
    // The dependency shows up as the members the function reads.
    assert!(
        has_edge(
            mapper,
            RelationshipKind::AccessesProperty,
            "include/user_payload.h::UserPayload::class"
        ),
        "have: {:?}",
        targets(mapper, RelationshipKind::AccessesProperty)
    );
}

#[test]
fn functions_are_extracted_with_their_names() {
    let repo = analyze("language_features");
    let mapper = file(&repo, "src/mapper.c");

    let functions: Vec<&str> = mapper
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Function)
        .map(|s| s.name.as_str())
        .collect();

    assert!(functions.contains(&"describe"), "got {functions:?}");
}
