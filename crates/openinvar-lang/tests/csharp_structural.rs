//! Tier 2, end to end — C#.
//!
//! Mirrors `java_structural.rs`. Half of what these assert is what Tier 2
//! *cannot* do: a structural language that quietly appeared to resolve across
//! files would be worse than one that openly does not, because every gate in
//! the later phases decides how much to trust a region by its tier.

#![cfg(feature = "csharp")]

use std::path::{Path, PathBuf};

use openinvar_core::ir::{Language, RepoIR, SymbolKind};
use openinvar_lang::spec::Tier;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/adapter-csharp/basic")
}

fn all_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(root).into_iter().flatten() {
        if e.path().is_file()
            && matches!(e.path().extension().and_then(|x| x.to_str()), Some("cs"))
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
    // The claim the tier rests on: no query file was written for C#, and
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
fn every_file_is_recorded_as_this_language() {
    let ir = analyzed();
    assert!(!ir.files.is_empty(), "no files were analysed at all");
    assert!(
        ir.files.iter().all(|f| f.language == Language::CSharp),
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
    // `Elsewhere` references a type declared in the other file. Tier 2 does
    // not resolve across files, so no edge may point out of the file it was
    // written in — silence here is the honest answer, and `status` reports the
    // tier so it is not mistaken for "nothing uses it".
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
    let spec = openinvar_lang::spec::for_language(&Language::CSharp)
        .expect("this build carries C#");
    assert_eq!(spec.tier(), Tier::Structural);
    assert!(
        !spec.tier().is_gate_safe(),
        "C# must never be gate-safe while it resolves nothing across files"
    );
}
