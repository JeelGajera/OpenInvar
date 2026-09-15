//! Tier 2, end to end — Lua.
//!
//! Mirrors `php_structural.rs`. Half of what these assert is what Tier 2
//! *cannot* do: a structural language that quietly appeared to resolve across
//! files would be worse than one that openly does not, because every gate in
//! the later phases decides how much to trust a region by its tier.

#![cfg(feature = "lua")]

use std::path::{Path, PathBuf};

use openinvar_core::ir::{Language, RepoIR, SymbolKind};
use openinvar_lang::spec::Tier;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/adapter-lua/basic")
}

fn all_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(root).into_iter().flatten() {
        if e.path().is_file()
            && matches!(e.path().extension().and_then(|x| x.to_str()), Some("lua"))
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
    // The claim the tier rests on: no query file was written for Lua, and
    // symbols still come out. A grammar whose `TAGS_QUERY` is absent or empty
    // would silently produce nothing, which is why this asserts a count rather
    // than that analysis merely succeeded.
    //
    // The names are functions rather than types: Lua has no class construct,
    // and `tree-sitter-lua`'s query defines `function` and `method` only. So
    // the symbol to look for is `handle`, not a `UserService` — asserting on a
    // type name here would be importing another language's shape.
    let ir = analyzed();
    let named: Vec<&str> = ir
        .files
        .iter()
        .flat_map(|f| f.symbols.iter())
        .filter(|s| s.kind != SymbolKind::Module)
        .map(|s| s.name.as_str())
        .collect();

    assert!(
        named.len() >= 3,
        "the tags query extracted almost nothing: {named:?}"
    );
    assert!(
        named.contains(&"handle"),
        "expected `handle` among the symbols, got {named:?}"
    );
}

#[test]
fn a_call_within_one_file_is_recorded() {
    // Lua's tags query carries `@reference.call`, so unlike Swift it produces
    // edges, and the tier's usual claim — symbols plus within-file references
    // — holds here in full. Worth asserting rather than assuming: the two
    // languages sit next to each other in the same table and differ on exactly
    // this, so which one is which cannot be left to memory.
    let ir = analyzed();
    let edges: usize = ir.files.iter().map(|f| f.relationships.len()).sum();

    assert!(
        edges > 0,
        "Lua recorded no references at all, which would put it where Swift is \
         and make the README's Tier 2 description wrong for it too"
    );
}

#[test]
fn every_file_is_recorded_as_this_language() {
    let ir = analyzed();
    assert!(!ir.files.is_empty(), "no files were analysed at all");
    assert!(
        ir.files.iter().all(|f| f.language == Language::Lua),
        "a file was attributed to the wrong language"
    );
}

#[test]
fn every_edge_is_structural() {
    // The property that makes broad coverage safe. A gate reads resolution per
    // edge, so a Tier 2 edge that claimed to be resolved would be acted on.
    let ir = analyzed();
    let resolutions: Vec<_> = ir
        .files
        .iter()
        .flat_map(|f| f.relationships.iter())
        .map(|r| (r.file.as_str(), r.line, r.resolution))
        .collect();

    assert!(!resolutions.is_empty(), "no edges at all to check");
    assert!(
        resolutions
            .iter()
            .all(|(_, _, res)| *res == openinvar_core::ir::Resolution::Structural),
        "a Tier 2 edge claimed to be resolved: {resolutions:?}"
    );
}

#[test]
fn nothing_resolves_across_files() {
    // `elsewhere.lua` does `require("user_service")` and calls through what
    // comes back. That value is whatever table the other file's last statement
    // produced, decided when it runs — so there is no cross-file answer to be
    // had here even in principle, and recording one would be a guess dressed
    // as a fact.
    let ir = analyzed();

    for file in &ir.files {
        for rel in &file.relationships {
            assert!(
                rel.to.starts_with(&file.file) || !rel.to.contains("::"),
                "{} points at another file: {}",
                file.file,
                rel.to
            );
        }
    }
}

#[test]
fn the_spec_reports_tier_two() {
    let spec = openinvar_lang::spec::for_language(&Language::Lua).expect("this build carries Lua");
    assert_eq!(spec.tier(), Tier::Structural);
    assert!(
        !spec.tier().is_gate_safe(),
        "Lua must never be gate-safe while it resolves nothing across files"
    );
}

#[test]
fn both_test_conventions_are_recognised() {
    // Lua has no single runner. busted looks in `spec/` for `*_spec.lua`;
    // luaunit files carry a `test` prefix or suffix. Supporting only one would
    // file half the ecosystem's tests as ordinary source.
    let spec = openinvar_lang::spec::for_language(&Language::Lua).expect("this build carries Lua");

    for path in [
        "spec/user_service_spec.lua",
        "tests/user_service_test.lua",
        "tests/test_user_service.lua",
        "spec/anything.lua",
    ] {
        assert!(spec.is_test_file(path), "{path} was not recognised as a test");
    }
    for path in [
        "src/user_service.lua",
        "src/latest.lua",
        "src/protest.lua",
    ] {
        assert!(
            !spec.is_test_file(path),
            "{path} was wrongly recognised as a test"
        );
    }
}
