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
fn test_gitignore_is_respected_by_default() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapter-ts/ignore_filters/src");

    let repo_ir =
        analyze_repo_with_config(&root, &ScanConfig::default_enabled()).expect("analysis succeeds");

    let files: Vec<String> = repo_ir.files.iter().map(|f| f.file.clone()).collect();
    assert!(files.iter().any(|f| f.ends_with("included.ts")));
    assert!(!files.iter().any(|f| f.ends_with("ignored.ts")));
    assert!(!files.iter().any(|f| f.contains("ignored_dir/")));
}

#[test]
fn test_include_and_exclude_patterns_are_applied() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapter-ts/ignore_filters/src");

    let cfg = ScanConfig {
        include_patterns: vec!["included_dir/**".to_string()],
        exclude_patterns: vec!["**/keep.ts".to_string()],
        respect_gitignore: false,
    };

    let repo_ir = analyze_repo_with_config(&root, &cfg).expect("analysis succeeds");
    let files: Vec<String> = repo_ir.files.iter().map(|f| f.file.clone()).collect();

    assert!(!files.iter().any(|f| f.ends_with("included.ts")));
    assert!(!files.iter().any(|f| f.ends_with("included_dir/keep.ts")));
}
