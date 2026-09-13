// Exercises the go module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "go")]

#[path = "common/go.rs"]
mod common;

use common::*;
use openinvar_core::ir::SymbolKind;

#[test]
fn a_package_symbol_is_created_once_per_package() {
    let repo = analyze("language_features");
    let package_ids = all_symbols(&repo, SymbolKind::Module);

    let store_packages: Vec<&String> = package_ids
        .iter()
        .filter(|id| id.ends_with("store::store::module"))
        .collect();

    // `store` spans store.go and helpers.go but is one package, so exactly one
    // node represents it.
    assert_eq!(
        store_packages.len(),
        1,
        "one package node per package, got {package_ids:?}"
    );
}

#[test]
fn names_declared_in_one_file_are_visible_from_another_in_the_same_package() {
    let repo = analyze("language_features");
    let helpers = file(&repo, "store/helpers.go");

    // `NewMemoryStore` in helpers.go returns `*MemoryStore` declared in
    // store.go. Go has no file-level visibility, so this must resolve.
    assert!(
        helpers
            .relationships
            .iter()
            .any(|r| r.to.ends_with("store/store.go::MemoryStore::class")),
        "package-scoped names must resolve across files, have: {:?}",
        helpers
            .relationships
            .iter()
            .map(|r| format!("{:?} -> {}", r.kind, r.to))
            .collect::<Vec<_>>()
    );
}
