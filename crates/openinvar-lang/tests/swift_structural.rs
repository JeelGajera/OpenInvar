//! Tier 2, end to end — Swift.
//!
//! Mirrors `php_structural.rs`. Half of what these assert is what Tier 2
//! *cannot* do: a structural language that quietly appeared to resolve across
//! files would be worse than one that openly does not, because every gate in
//! the later phases decides how much to trust a region by its tier.

#![cfg(feature = "swift")]

use std::path::{Path, PathBuf};

use openinvar_core::ir::{Language, RepoIR, SymbolKind};
use openinvar_lang::spec::Tier;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/adapter-swift/basic")
}

fn all_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(root).into_iter().flatten() {
        if e.path().is_file()
            && matches!(e.path().extension().and_then(|x| x.to_str()), Some("swift"))
        {
            out.push(e.path().to_path_buf());
        }
    }
    out.sort();
    out
}

fn analyzed() -> RepoIR {
    let root = fixture_root();
    openinvar_lang::dispatch::analyze_files(&root, &all_files(&root)).expect("analysis must succeed")
}

#[test]
fn the_grammars_own_tags_query_extracts_symbols() {
    // The claim the tier rests on: no query file was written for Swift, and
    // symbols still come out. A grammar whose `TAGS_QUERY` is absent or empty
    // would silently produce nothing, which is why this asserts a count rather
    // than that analysis merely succeeded.
    let ir = analyzed();
    let named: Vec<&str> = ir
        .files
        .iter()
        .flat_map(|f| f.symbols.iter())
        .filter(|s| s.kind != SymbolKind::Module)
        .map(|s| s.name.as_str())
        .collect();

    assert!(
        named.len() >= 4,
        "the tags query extracted almost nothing: {named:?}"
    );
    assert!(
        named.contains(&"UserService"),
        "expected UserService among the symbols, got {named:?}"
    );
}

#[test]
fn declaration_forms_peculiar_to_swift_are_all_found() {
    // Swift spreads declarations across forms that other languages fold into
    // one or two: `protocol` and `struct` are not `class`, and a grammar's tags
    // query is free to cover some and not others. Asserting only on the class
    // would pass while two thirds of a Swift file went unindexed.
    let ir = analyzed();
    let named: Vec<&str> = ir
        .files
        .iter()
        .flat_map(|f| f.symbols.iter())
        .map(|s| s.name.as_str())
        .collect();

    for declaration in ["Auditing", "AuditLog", "UserService"] {
        assert!(
            named.contains(&declaration),
            "`{declaration}` was not extracted: {named:?}"
        );
    }
}

#[test]
fn every_file_is_recorded_as_this_language() {
    let ir = analyzed();
    assert!(!ir.files.is_empty(), "no files were analysed at all");
    assert!(
        ir.files.iter().all(|f| f.language == Language::Swift),
        "a file was attributed to the wrong language"
    );
}

#[test]
fn swift_records_symbols_but_no_references() {
    // The limit that separates Swift from the other Tier 2 languages, and the
    // reason the README gives it a note of its own rather than folding it in.
    //
    // A Tier 2 language is analysed entirely through the `tags.scm` its grammar
    // ships, and `tree-sitter-swift`'s contains only `@definition` captures —
    // no `@reference.call`, which Ruby, PHP and Lua all provide. So Swift
    // yields symbols and no edges whatsoever: a definition can be located and a
    // file's declarations listed, but intra-file usage cannot be seen.
    //
    // Asserted rather than left implicit, because the two tests this replaces
    // would both have passed vacuously. "Every edge is structural" and "nothing
    // resolves across files" are trivially true of a language with no edges,
    // and a reader seeing them pass would reasonably conclude Swift had been
    // checked for something it never was.
    let ir = analyzed();

    let symbols = ir
        .files
        .iter()
        .flat_map(|f| f.symbols.iter())
        .filter(|s| s.kind != SymbolKind::Module)
        .count();
    assert!(symbols > 0, "Swift produced no symbols either, which is a break");

    let edges: Vec<_> = ir
        .files
        .iter()
        .flat_map(|f| f.relationships.iter())
        .map(|r| (r.file.as_str(), r.line, r.kind.clone()))
        .collect();
    assert!(
        edges.is_empty(),
        "Swift now records references, so its grammar has gained `@reference` \
         captures. That is an improvement, and it means the README's Swift \
         note, this test, and the tier-2 description all need updating — \
         together with a check that every new edge is structural: {edges:?}"
    );
}

#[test]
fn the_spec_reports_tier_two() {
    let spec =
        openinvar_lang::spec::for_language(&Language::Swift).expect("this build carries Swift");
    assert_eq!(spec.tier(), Tier::Structural);
    assert!(
        !spec.tier().is_gate_safe(),
        "Swift must never be gate-safe while it resolves nothing across files"
    );
}

#[test]
fn swiftpm_test_files_are_recognised() {
    // SwiftPM's suffix is plural — `UserServiceTests.swift` — where PHPUnit's
    // is singular. Sharing one rule between the two adapters would silently
    // miss one ecosystem's convention, so each spells out its own.
    let spec =
        openinvar_lang::spec::for_language(&Language::Swift).expect("this build carries Swift");

    for path in [
        "Tests/AppTests/UserServiceTests.swift",
        "Sources/App/UserServiceTests.swift",
        "Tests/AppTests/AnyName.swift",
    ] {
        assert!(spec.is_test_file(path), "{path} was not recognised as a test");
    }
    for path in [
        "Sources/App/UserService.swift",
        "Sources/App/Contest.swift",
        "Sources/App/Latest.swift",
    ] {
        assert!(
            !spec.is_test_file(path),
            "{path} was wrongly recognised as a test"
        );
    }
}
