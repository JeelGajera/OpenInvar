// Exercises the typescript module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "typescript")]

use std::path::PathBuf;

use openinvar_core::ir::RelationshipKind;
use openinvar_core::scan::{walk_source_files_with_config, ScanConfig};
use openinvar_lang::lang::typescript::analyze_files;
use openinvar_lang::lang::typescript::language::is_supported_source_file;

fn analyze_repo_at(
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
fn test_tsconfig_alias_resolves() {
    // The root must be the project root (where tsconfig.json lives),
    // not the src/ subdirectory.
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/adapter-ts/tsconfig_paths");
    let repo_ir = analyze_repo_at(&root).expect("analysis succeeds");

    let app = repo_ir
        .files
        .iter()
        .find(|f| f.file.ends_with("app.ts"))
        .expect("app.ts exists");

    // @utils/helpers should resolve to src/utils/helpers.ts::helper
    let helper_import = app
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::Imports && r.to.contains("helper"))
        .expect("@utils/helpers alias should resolve");
    assert!(
        helper_import.to.contains("helpers.ts"),
        "Should resolve to helpers.ts, got: {}",
        helper_import.to
    );

    // @/config should resolve to src/config.ts::AppConfig
    let config_import = app
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::Imports && r.to.contains("AppConfig"));
    assert!(
        config_import.is_some(),
        "Should resolve @/config alias to AppConfig, got imports: {:?}",
        app.relationships
            .iter()
            .filter(|r| r.kind == RelationshipKind::Imports)
            .map(|r| &r.to)
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_tsconfig_missing_does_not_crash() {
    // This fixture has no tsconfig.json — should work fine (all non-relative become external)
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapter-ts/external_imports/src");
    let repo_ir = analyze_repo_at(&root).expect("analysis succeeds without tsconfig");
    assert!(!repo_ir.files.is_empty());
}
