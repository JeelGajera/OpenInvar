// Exercises the typescript module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "typescript")]

use std::path::PathBuf;

use openinvar_core::ir::DiagnosticLevel;
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
fn test_cyclic_barrel_does_not_hang() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapter-ts/cyclic_barrel/src");
    // This must complete without infinite recursion
    let repo_ir = analyze_repo(&root).expect("analysis succeeds even with cycle");

    // Verify the analysis produced some files (didn't crash)
    assert!(
        !repo_ir.files.is_empty(),
        "Should produce file analysis results"
    );
}

#[test]
fn test_cyclic_barrel_emits_warning() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapter-ts/cyclic_barrel/src");
    let repo_ir = analyze_repo(&root).expect("analysis succeeds");

    // Collect all diagnostics
    let all_diags: Vec<_> = repo_ir
        .files
        .iter()
        .flat_map(|f| f.diagnostics.iter())
        .collect();

    // There should be at least one warning about the cycle or unresolved symbol
    let warnings: Vec<_> = all_diags
        .iter()
        .filter(|d| d.level == DiagnosticLevel::Warning)
        .collect();
    assert!(
        !warnings.is_empty(),
        "Cyclic barrel should produce at least one warning diagnostic"
    );
}
