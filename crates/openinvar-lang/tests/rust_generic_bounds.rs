// Exercises the rust module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "rust")]

#[path = "common/rust.rs"]
mod common;

use common::*;
use openinvar_core::ir::SymbolKind;

#[test]
fn generic_methods_are_extracted_with_their_owning_type() {
    let repo = analyze("language_features");
    let reporting = file(&repo, "services/reporting.rs");

    // `pub fn label<T: Identify>(&self, subject: &T)`
    assert!(
        reporting
            .symbols
            .iter()
            .any(|s| s.kind == SymbolKind::Method && s.name == "label"),
        "a generic method is still a method, got: {:?}",
        symbol_names(reporting, SymbolKind::Method)
    );
}

#[test]
fn generic_type_parameters_are_not_mistaken_for_repository_types() {
    let repo = analyze("language_features");
    let reporting = file(&repo, "services/reporting.rs");

    // `subject: &T` binds to the type parameter `T`, which is not a symbol
    // anywhere. It must not resolve to something unrelated, and must not
    // produce an edge into the graph.
    assert!(
        !reporting
            .relationships
            .iter()
            .any(|r| r.to.ends_with("::T::class")),
        "a type parameter must not resolve to a concrete symbol"
    );
}
