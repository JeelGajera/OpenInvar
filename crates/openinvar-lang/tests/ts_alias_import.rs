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

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/alias-import-bug/src")
}

#[test]
fn test_alias_import_fixture_is_resolved_with_property_access() {
    let repo_ir = analyze_repo(&fixture_root()).expect("repo analysis must succeed");

    let mapper_file = repo_ir
        .files
        .iter()
        .find(|f| f.file.ends_with("mappers/deep/view_model_mapper.ts"))
        .expect("mapper file exists");

    let rel = mapper_file
        .relationships
        .iter()
        .find(|r| {
            r.kind == RelationshipKind::Imports && r.alias.as_deref() == Some("ResponseModel")
        })
        .expect("aliased import relationship exists");

    assert!(rel
        .to
        .ends_with("models/user_payload.ts::UserPayload::class"));
    assert_eq!(
        rel.properties_accessed,
        vec![
            "status".to_string(),
            "timestamp".to_string(),
            "userId".to_string()
        ]
    );

    let prop_rel = mapper_file
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::AccessesProperty)
        .expect("property access relationship exists");
    assert!(prop_rel
        .to
        .ends_with("models/user_payload.ts::UserPayload::class"));
}
