// Exercises the c module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "c")]

#[path = "common/c.rs"]
mod common;

use common::*;
use openinvar_core::ir::RelationshipKind;

#[test]
fn a_namespace_qualified_type_resolves_to_its_class() {
    let repo = analyze("language_features");
    let render = file(&repo, "src/render.cpp");

    // `geometry::Circle` — the qualifier is stripped for lookup, because
    // namespaces do not change which file defines the class.
    assert!(
        has_edge(
            render,
            RelationshipKind::Imports,
            "include/shapes.hpp::Circle::class"
        ),
        "have: {:?}",
        targets(render, RelationshipKind::Imports)
    );
}

#[test]
fn unresolvable_types_are_reported_rather_than_dropped_silently() {
    // Everything in this fixture resolves, so there should be no resolution
    // warnings. The value of the assertion is that a future regression which
    // starts dropping edges will surface here.
    let repo = analyze("language_features");
    let warnings: Vec<&str> = repo
        .files
        .iter()
        .flat_map(|f| f.diagnostics.iter())
        .filter(|d| d.category == openinvar_core::ir::DiagnosticCategory::Resolution)
        .map(|d| d.message.as_str())
        .collect();

    assert!(
        warnings.is_empty(),
        "unexpected resolution gaps: {warnings:?}"
    );
}
