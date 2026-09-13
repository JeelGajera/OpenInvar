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
fn test_deep_barrel_3_hop_resolves() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/adapter-ts/deep_barrel/src");
    let repo_ir = analyze_repo(&root).expect("analysis succeeds");

    let app = repo_ir
        .files
        .iter()
        .find(|f| f.file.ends_with("app.ts"))
        .expect("app.ts exists");

    // renderBlockquote should resolve through:
    // app.ts -> lib/index.ts -> lib/renderers/index.ts -> lib/renderers/blockquote.ts
    let bq_import = app
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::Imports && r.to.contains("renderBlockquote"))
        .expect("renderBlockquote import should resolve");
    assert!(
        bq_import.to.contains("blockquote.ts"),
        "Should resolve to blockquote.ts through 3-hop chain, got: {}",
        bq_import.to
    );
}

#[test]
fn test_deep_barrel_no_unresolved_errors() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/adapter-ts/deep_barrel/src");
    let repo_ir = analyze_repo(&root).expect("analysis succeeds");

    let app = repo_ir
        .files
        .iter()
        .find(|f| f.file.ends_with("app.ts"))
        .expect("app.ts exists");

    let unresolved: Vec<_> = app
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("unable to resolve"))
        .collect();
    assert!(
        unresolved.is_empty(),
        "3-hop barrel should resolve without errors, got: {:?}",
        unresolved
    );
}
