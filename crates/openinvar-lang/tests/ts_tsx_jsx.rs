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

#[test]
fn test_tsx_and_jsx_files_parse_without_errors() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/adapter-ts/tsx_jsx/src");
    let repo_ir = analyze_repo(&root).expect("repo analysis must succeed");

    let tsx = repo_ir
        .files
        .iter()
        .find(|f| f.file.ends_with("component.tsx"))
        .expect("tsx file exists");
    let jsx = repo_ir
        .files
        .iter()
        .find(|f| f.file.ends_with("view.jsx"))
        .expect("jsx file exists");

    assert!(
        !tsx.diagnostics
            .iter()
            .any(|d| d.message.starts_with("line ")),
        "tsx syntax parse errors: {:?}",
        tsx.diagnostics
    );
    assert!(
        !jsx.diagnostics
            .iter()
            .any(|d| d.message.starts_with("line ")),
        "jsx syntax parse errors: {:?}",
        jsx.diagnostics
    );
    assert!(tsx.symbols.iter().any(|s| s.name == "Header"));
    assert!(jsx.symbols.iter().any(|s| s.name == "View"));
}
