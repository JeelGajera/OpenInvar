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
fn test_multiline_import_and_property_access_are_extracted() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapter-ts/multiline_import/src");
    let repo_ir = analyze_repo(&root).expect("repo analysis must succeed");

    let use_file = repo_ir
        .files
        .iter()
        .find(|f| f.file.ends_with("use.ts"))
        .expect("use.ts exists");

    let import_rel = use_file
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::Imports && r.alias.as_deref() == Some("AuthSession"))
        .expect("multiline import relationship exists");
    assert!(import_rel.to.ends_with("model.ts::Session::interface"));

    let prop_rel = use_file
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::AccessesProperty)
        .expect("property relationship exists");
    assert!(prop_rel.to.ends_with("model.ts::Session::interface"));
    assert_eq!(
        prop_rel.properties_accessed,
        vec!["token".to_string(), "userId".to_string()]
    );
}
