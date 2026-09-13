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
fn test_builtin_types_produce_no_property_access_errors() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapter-ts/builtin_types/src");
    let repo_ir = analyze_repo(&root).expect("analysis succeeds");

    let app = repo_ir
        .files
        .iter()
        .find(|f| f.file.ends_with("app.ts"))
        .expect("app.ts exists");

    // No diagnostics about built-in types (string, Array, Partial, number)
    let builtin_errors: Vec<_> = app
        .diagnostics
        .iter()
        .filter(|d| {
            d.message.contains("'string'")
                || d.message.contains("'Array'")
                || d.message.contains("'Partial'")
                || d.message.contains("'number'")
        })
        .collect();
    assert!(
        builtin_errors.is_empty(),
        "Built-in types should not produce diagnostics, got: {:?}",
        builtin_errors
    );
}

#[test]
fn test_local_type_property_access_still_works() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapter-ts/builtin_types/src");
    let repo_ir = analyze_repo(&root).expect("analysis succeeds");

    let app = repo_ir
        .files
        .iter()
        .find(|f| f.file.ends_with("app.ts"))
        .expect("app.ts exists");

    // Config is a local interface — property access on it (cfg.host) should work
    let config_access = app.relationships.iter().find(|r| {
        r.kind == RelationshipKind::AccessesProperty
            && r.properties_accessed.contains(&"host".to_string())
    });
    assert!(
        config_access.is_some(),
        "Property access on local type Config should be tracked"
    );
}
