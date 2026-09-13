// Exercises the typescript module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "typescript")]

use std::path::PathBuf;

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

#[allow(dead_code)]
fn analyze_repo_with_config(
    root: &std::path::Path,
    config: &ScanConfig,
) -> Result<openinvar_core::ir::RepoIR, openinvar_lang::lang::typescript::AdapterTsError> {
    let files = walk_source_files_with_config(root, config, is_supported_source_file).unwrap();
    analyze_files(root, &files)
}

use openinvar_core::ir::RelationshipKind;

#[test]
fn test_reexports_and_barrel_are_resolved() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/adapter-ts/re_exports/src");
    let repo_ir = analyze_repo(&root).expect("repo analysis must succeed");

    let index_file = repo_ir
        .files
        .iter()
        .find(|f| f.file.ends_with("index.ts"))
        .expect("index file exists");

    let named_reexport = index_file
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::ReExports && r.alias.as_deref() == Some("PublicUser"))
        .expect("named re-export with alias exists");
    assert!(named_reexport
        .to
        .ends_with("user_payload.ts::UserPayload::class"));

    let barrel_reexports: Vec<_> = index_file
        .relationships
        .iter()
        .filter(|r| r.kind == RelationshipKind::ReExports && r.context.contains("export *"))
        .collect();
    assert!(!barrel_reexports.is_empty());
}
