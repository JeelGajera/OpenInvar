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
fn test_external_imports_produce_no_errors() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapter-ts/external_imports/src");
    let repo_ir = analyze_repo(&root).expect("analysis succeeds");

    let all_diagnostics: Vec<_> = repo_ir
        .files
        .iter()
        .flat_map(|f| f.diagnostics.iter())
        .collect();

    let external_errors: Vec<_> = all_diagnostics
        .iter()
        .filter(|d| {
            d.message.contains("react")
                || d.message.contains("path")
                || d.message.contains("@quantajs/core")
        })
        .collect();
    assert!(
        external_errors.is_empty(),
        "External imports should not produce errors, got: {:?}",
        external_errors
    );
}

#[test]
fn test_local_import_still_resolves() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapter-ts/external_imports/src");
    let repo_ir = analyze_repo(&root).expect("analysis succeeds");

    let app_file = repo_ir
        .files
        .iter()
        .find(|f| f.file.ends_with("app.ts"))
        .expect("app.ts exists");

    let local_import = app_file
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::Imports && r.to.contains("localHelper"))
        .expect("local import to localHelper should resolve");

    assert!(local_import.to.contains("utils.ts"));
}

#[test]
fn test_external_imports_create_dependency_edges() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapter-ts/external_imports/src");
    let repo_ir = analyze_repo(&root).expect("analysis succeeds");

    let app_file = repo_ir
        .files
        .iter()
        .find(|f| f.file.ends_with("app.ts"))
        .expect("app.ts exists");

    // react import should create an edge to ext::react::package
    let react_edge = app_file
        .relationships
        .iter()
        .find(|r| r.to == "ext::react::package");
    assert!(
        react_edge.is_some(),
        "Should have dependency edge to ext::react::package, got edges: {:?}",
        app_file
            .relationships
            .iter()
            .map(|r| &r.to)
            .collect::<Vec<_>>()
    );

    // path (node builtin) should create an edge to ext::path::package
    let path_edge = app_file
        .relationships
        .iter()
        .find(|r| r.to == "ext::path::package");
    assert!(
        path_edge.is_some(),
        "Should have dependency edge to ext::path::package"
    );

    // @quantajs/core (scoped) should create edge to ext::@quantajs/core::package
    let scoped_edge = app_file
        .relationships
        .iter()
        .find(|r| r.to == "ext::@quantajs/core::package");
    assert!(
        scoped_edge.is_some(),
        "Should have dependency edge to ext::@quantajs/core::package"
    );
}
