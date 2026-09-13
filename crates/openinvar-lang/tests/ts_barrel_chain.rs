// Exercises the typescript module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "typescript")]

use std::path::PathBuf;

use openinvar_core::ir::RelationshipKind;
use openinvar_core::scan::{walk_source_files_with_config, ScanConfig};
use openinvar_lang::lang::typescript::analyze_files;
use openinvar_lang::lang::typescript::language::is_supported_source_file;

#[allow(dead_code)]
fn analyze_repo(
    root: &std::path::Path,
) -> Result<openinvar_core::ir::RepoIR, openinvar_lang::lang::typescript::AdapterTsError> {
    let files = walk_source_files_with_config(
        root,
        &ScanConfig::default_enabled(),
        is_supported_source_file,
    )
    .unwrap();
    analyze_files(root, &files)
}

#[test]
fn test_barrel_chain_resolves_named_reexports() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapter-ts/barrel_chain/src");
    let repo_ir = analyze_repo(&root).expect("analysis succeeds");

    let app = repo_ir
        .files
        .iter()
        .find(|f| f.file.ends_with("app.ts"))
        .expect("app.ts exists");

    // renderBlockquote should resolve through components/index.ts -> components/blockquote.ts
    let bq_import = app
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::Imports && r.to.contains("renderBlockquote"))
        .expect("renderBlockquote import should resolve");
    assert!(
        bq_import.to.contains("blockquote.ts"),
        "renderBlockquote should resolve to blockquote.ts, got: {}",
        bq_import.to
    );

    // renderCodeBlock should resolve through components/index.ts -> components/code.ts
    let cb_import = app
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::Imports && r.to.contains("renderCodeBlock"))
        .expect("renderCodeBlock import should resolve");
    assert!(
        cb_import.to.contains("code.ts"),
        "renderCodeBlock should resolve to code.ts, got: {}",
        cb_import.to
    );
}

#[test]
fn test_barrel_chain_no_unresolved_symbol_errors() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapter-ts/barrel_chain/src");
    let repo_ir = analyze_repo(&root).expect("analysis succeeds");

    let app = repo_ir
        .files
        .iter()
        .find(|f| f.file.ends_with("app.ts"))
        .expect("app.ts exists");

    let unresolved: Vec<_> = app
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("unable to resolve symbol"))
        .collect();
    assert!(
        unresolved.is_empty(),
        "Barrel chain should resolve without errors, got: {:?}",
        unresolved
    );
}
