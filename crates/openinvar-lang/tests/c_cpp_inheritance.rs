// Exercises the c module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "c")]

#[path = "common/c.rs"]
mod common;

use common::*;
use openinvar_core::ir::{RelationshipKind, SymbolKind};

#[test]
fn a_base_class_produces_an_extends_edge() {
    let repo = analyze("language_features");
    let shapes = file(&repo, "include/shapes.hpp");

    let edge = shapes
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::Extends)
        .expect("`class Circle : public Shape` should be recorded");

    assert!(edge.from.ends_with("Circle::class"), "got {}", edge.from);
    assert!(edge.to.ends_with("Shape::class"), "got {}", edge.to);
}

#[test]
fn both_classes_are_extracted_as_distinct_symbols() {
    let repo = analyze("language_features");
    let shapes = file(&repo, "include/shapes.hpp");

    let classes: Vec<&str> = shapes
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Class)
        .map(|s| s.name.as_str())
        .collect();

    assert!(classes.contains(&"Shape"), "got {classes:?}");
    assert!(classes.contains(&"Circle"), "got {classes:?}");
}

#[test]
fn a_cpp_header_is_parsed_with_the_cpp_grammar() {
    // A `.hpp` is unambiguous, but the classes only appear if the C++ grammar
    // was used — the C grammar loses them to error recovery.
    let repo = analyze("language_features");
    let shapes = file(&repo, "include/shapes.hpp");

    assert!(
        shapes.symbols.iter().any(|s| s.name == "Circle"),
        "C++ classes require the C++ grammar"
    );
    assert!(
        shapes
            .diagnostics
            .iter()
            .all(|d| d.level != openinvar_core::ir::DiagnosticLevel::Error),
        "a valid C++ header should parse cleanly: {:?}",
        shapes.diagnostics
    );
}
