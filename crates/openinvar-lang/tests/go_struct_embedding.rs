// Exercises the go module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "go")]

#[path = "common/go.rs"]
mod common;

use common::*;
use openinvar_core::ir::SymbolKind;

#[test]
fn embedded_structs_are_extracted_as_distinct_types() {
    let repo = analyze("language_features");
    let user = file(&repo, "models/user.go");

    let classes: Vec<&str> = user
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Class)
        .map(|s| s.name.as_str())
        .collect();

    // `UserPayload` embeds `Base`; both are real types in the package.
    assert!(classes.contains(&"Base"), "got {classes:?}");
    assert!(classes.contains(&"UserPayload"), "got {classes:?}");
}

#[test]
fn embedding_does_not_merge_the_two_types_into_one_symbol() {
    let repo = analyze("language_features");
    let user = file(&repo, "models/user.go");

    let base = user.symbols.iter().find(|s| s.name == "Base").unwrap();
    let payload = user
        .symbols
        .iter()
        .find(|s| s.name == "UserPayload")
        .unwrap();

    assert_ne!(
        base.id, payload.id,
        "an embedded type keeps its own identity in the graph"
    );
}
