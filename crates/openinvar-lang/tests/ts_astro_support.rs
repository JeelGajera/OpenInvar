// Exercises the typescript module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "typescript")]

use std::path::PathBuf;

use openinvar_core::ir::{DiagnosticLevel, RelationshipKind};
use openinvar_core::scan::{walk_source_files_with_config, ScanConfig};
use openinvar_lang::lang::typescript::analyze_files;
use openinvar_lang::lang::typescript::language::is_supported_source_file;

fn analyze(root: &std::path::Path) -> openinvar_core::ir::RepoIR {
    let files = walk_source_files_with_config(
        root,
        &ScanConfig::default_enabled(),
        is_supported_source_file,
    )
    .expect("scan succeeds");
    analyze_files(root, &files).expect("analysis succeeds")
}

#[test]
fn test_astro_frontmatter_imports_and_lines_are_supported() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapter-ts/astro_component/src");
    let repo_ir = analyze(&root);

    assert!(
        repo_ir
            .files
            .iter()
            .any(|f| f.file.ends_with("pages/index.astro")),
        "pages/index.astro should be indexed"
    );
    assert!(
        repo_ir
            .files
            .iter()
            .any(|f| f.file.ends_with("layouts/Layout.astro")),
        "layouts/Layout.astro should be indexed"
    );

    let page = repo_ir
        .files
        .iter()
        .find(|f| f.file.ends_with("pages/index.astro"))
        .expect("pages/index.astro exists");

    let card_import = page.relationships.iter().find(|r| {
        r.kind == RelationshipKind::Imports
            && r.to.contains("Card")
            && r.to.contains("components/Card.ts")
    });
    assert!(
        card_import.is_some(),
        "index.astro should resolve Card import to components/Card.ts, got: {:?}",
        page.relationships
            .iter()
            .map(|r| r.to.as_str())
            .collect::<Vec<_>>()
    );

    let parse_errors: Vec<_> = page
        .diagnostics
        .iter()
        .filter(|d| d.level == DiagnosticLevel::Error)
        .collect();
    assert!(
        parse_errors.is_empty(),
        "Astro template should be blanked without parser errors, got: {:?}",
        parse_errors
    );

    if let Some(imp) = card_import {
        assert!(
            imp.line >= 2,
            "Astro frontmatter import line should stay near top of file, got line {}",
            imp.line
        );
    }
}
