// Exercises the typescript module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "typescript")]

use std::path::PathBuf;

use openinvar_core::ir::{DiagnosticLevel, RelationshipKind};
use openinvar_core::scan::{walk_source_files_with_config, ScanConfig};
use openinvar_lang::lang::typescript::analyze_files;
use openinvar_lang::lang::typescript::language::is_supported_source_file;

fn analyze(root: &std::path::Path) -> openinvar_core::ir::RepoIR {
    let files = walk_source_files_with_config(
        root,
        &ScanConfig::default_enabled(),
        is_supported_source_file,
    )
    .expect("scan succeeds");
    analyze_files(root, &files).expect("analysis succeeds")
}

#[test]
fn test_svelte_files_are_indexed_and_imports_resolve_without_parse_errors() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapter-ts/svelte_component/src");
    let repo_ir = analyze(&root);

    assert!(
        repo_ir.files.iter().any(|f| f.file.ends_with("App.svelte")),
        "App.svelte should be indexed"
    );

    let app = repo_ir
        .files
        .iter()
        .find(|f| f.file.ends_with("App.svelte"))
        .expect("App.svelte exists");

    let store_import = app.relationships.iter().find(|r| {
        r.kind == RelationshipKind::Imports
            && r.to.contains("UserStore")
            && r.to.contains("stores/UserStore.ts")
    });
    assert!(
        store_import.is_some(),
        "App.svelte should resolve UserStore import to stores/UserStore.ts, got: {:?}",
        app.relationships
            .iter()
            .map(|r| r.to.as_str())
            .collect::<Vec<_>>()
    );

    let parse_errors: Vec<_> = app
        .diagnostics
        .iter()
        .filter(|d| d.level == DiagnosticLevel::Error)
        .collect();
    assert!(
        parse_errors.is_empty(),
        "Svelte template should be blanked without parser errors, got: {:?}",
        parse_errors
    );
}
