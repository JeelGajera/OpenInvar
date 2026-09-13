//! Shared fixture loading for the Python adapter's integration tests.

// Each test binary in this crate compiles the whole module but uses only
// part of it; the unused remainder is not dead code from the crate's view.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use openinvar_core::ir::{FileIR, RelationshipKind, RepoIR, SymbolKind};

pub fn fixture_root(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/adapter-py/{name}"))
}

pub fn python_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root).into_iter().flatten() {
        if entry.path().is_file()
            && matches!(
                entry.path().extension().and_then(|e| e.to_str()),
                Some("py") | Some("pyi")
            )
        {
            out.push(entry.path().to_path_buf());
        }
    }
    out.sort();
    out
}

pub fn analyze(name: &str) -> RepoIR {
    let root = fixture_root(name);
    let files = python_files(&root);
    assert!(!files.is_empty(), "fixture '{name}' has no Python files");
    openinvar_lang::lang::python::analyze_files(&root, &files)
        .unwrap_or_else(|e| panic!("analysis of '{name}' failed: {e}"))
}

pub fn file<'a>(repo: &'a RepoIR, suffix: &str) -> &'a FileIR {
    repo.files
        .iter()
        .find(|f| f.file.ends_with(suffix))
        .unwrap_or_else(|| {
            panic!(
                "no file ending in '{suffix}'; have: {:?}",
                repo.files.iter().map(|f| &f.file).collect::<Vec<_>>()
            )
        })
}

pub fn has_edge(file: &FileIR, kind: RelationshipKind, target_suffix: &str) -> bool {
    file.relationships
        .iter()
        .any(|r| r.kind == kind && r.to.ends_with(target_suffix))
}

pub fn targets(file: &FileIR, kind: RelationshipKind) -> Vec<String> {
    file.relationships
        .iter()
        .filter(|r| r.kind == kind)
        .map(|r| r.to.clone())
        .collect()
}

pub fn properties_for(file: &FileIR, kind: RelationshipKind, target_suffix: &str) -> Vec<String> {
    file.relationships
        .iter()
        .find(|r| r.kind == kind && r.to.ends_with(target_suffix))
        .map(|r| r.properties_accessed.clone())
        .unwrap_or_else(|| {
            panic!(
                "no {kind:?} edge ending in '{target_suffix}'; have: {:?}",
                file.relationships
                    .iter()
                    .map(|r| format!("{:?} -> {}", r.kind, r.to))
                    .collect::<Vec<_>>()
            )
        })
}

pub fn symbol_kind(file: &FileIR, name: &str) -> SymbolKind {
    file.symbols
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| {
            panic!(
                "no symbol '{name}'; have: {:?}",
                file.symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
            )
        })
        .kind
        .clone()
}
