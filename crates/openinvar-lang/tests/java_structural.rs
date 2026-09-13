//! Tier 2, end to end.
//!
//! Java is the demonstration that a structural language costs a dependency, a
//! feature flag and a spec — no parser, no extractor, no resolver, and no
//! query files, because the grammar already ships a `tags.scm`.
//!
//! Half of what these assert is what Tier 2 *cannot* do. A structural language
//! that quietly appeared to resolve across files would be worse than one that
//! openly does not, because every gate in the later phases decides how much to
//! trust a region by its tier.

#![cfg(feature = "java")]

use std::path::{Path, PathBuf};

use openinvar_core::ir::{FileIR, RelationshipKind, RepoIR, SymbolKind};
use openinvar_lang::spec::Tier;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/adapter-java/basic")
}

fn all_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(root).into_iter().flatten() {
        if e.path().is_file() && e.path().extension().and_then(|x| x.to_str()) == Some("java") {
            out.push(e.path().to_path_buf());
        }
    }
    out.sort();
    out
}

fn analyzed() -> RepoIR {
    let root = fixture_root();
    openinvar_lang::analyze_files(&root, &all_files(&root)).expect("java analysis must succeed")
}

fn file<'a>(ir: &'a RepoIR, suffix: &str) -> &'a FileIR {
    ir.files
        .iter()
        .find(|f| f.file.ends_with(suffix))
        .unwrap_or_else(|| panic!("{suffix} is present"))
}

#[test]
fn java_is_registered_as_a_structural_language() {
    let spec = openinvar_lang::spec::for_language(&openinvar_core::ir::Language::Java)
        .expect("java is registered");
    assert_eq!(spec.tier(), Tier::Structural);
    assert!(
        !spec.tier().is_gate_safe(),
        "a structural language must never be gate-safe"
    );
    assert_eq!(spec.extensions(), &["java"]);
}

#[test]
fn the_grammars_own_tags_query_drives_extraction() {
    // No query file was written for Java. If this ever returns None the
    // structural path silently analyses nothing.
    let spec = openinvar_lang::spec::for_language(&openinvar_core::ir::Language::Java).unwrap();
    assert!(spec.tags_query().is_some());
    assert!(spec.grammar().is_some());
}

#[test]
fn symbols_are_extracted_without_a_hand_written_extractor() {
    let ir = analyzed();
    let service = file(&ir, "UserService.java");

    let found: Vec<(&str, &SymbolKind)> = service
        .symbols
        .iter()
        .map(|s| (s.name.as_str(), &s.kind))
        .collect();

    assert!(
        found.contains(&("UserService", &SymbolKind::Class)),
        "expected the class: {found:?}"
    );
    assert!(
        found.contains(&("Auditable", &SymbolKind::Interface)),
        "expected the interface: {found:?}"
    );
    assert!(
        found.contains(&("handle", &SymbolKind::Method)),
        "expected the method: {found:?}"
    );
}

#[test]
fn intra_file_references_become_edges() {
    let ir = analyzed();
    let service = file(&ir, "UserService.java");

    let kinds: Vec<&RelationshipKind> = service.relationships.iter().map(|r| &r.kind).collect();
    assert!(
        kinds.contains(&&RelationshipKind::Implements),
        "`implements Auditable` is visible in this file: {kinds:?}"
    );
    assert!(
        kinds.contains(&&RelationshipKind::Calls),
        "a call to a method declared in this file: {kinds:?}"
    );

    for rel in &service.relationships {
        assert!(
            rel.alias.is_none(),
            "structural analysis has no import table, so it cannot know an alias"
        );
    }
}

#[test]
fn a_cross_file_reference_records_nothing() {
    // The defining limit of Tier 2, and the reason it is not gate-safe.
    // `Elsewhere` constructs and calls `UserService`, which is declared in
    // another file. A tags query reports that a call happened; it does not say
    // to which `UserService`. Guessing repository-wide by leaf name is exactly
    // the bug 0.2.0 fixed in the Rust adapter, and doing it here would
    // reintroduce it across every structural language at once.
    let ir = analyzed();
    let elsewhere = file(&ir, "Elsewhere.java");

    for rel in &elsewhere.relationships {
        assert!(
            rel.to.contains("Elsewhere.java"),
            "a structural edge must stay inside its own file, got {}",
            rel.to
        );
    }
}

#[test]
fn structural_analysis_is_deterministic() {
    // It runs files in parallel and collects matches from a query cursor,
    // neither of which orders anything on its own.
    let first = analyzed();
    let second = analyzed();

    let render = |ir: &RepoIR| {
        ir.files
            .iter()
            .map(|f| {
                format!(
                    "{}|{}|{}",
                    f.file,
                    f.symbols
                        .iter()
                        .map(|s| s.id.clone())
                        .collect::<Vec<_>>()
                        .join(","),
                    f.relationships
                        .iter()
                        .map(|r| format!("{}->{}@{}", r.from, r.to, r.line))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    assert_eq!(render(&first), render(&second));
}
