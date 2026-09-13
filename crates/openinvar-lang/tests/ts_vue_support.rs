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
fn test_vue_files_index_imports_and_lines_are_supported() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapter-ts/vue_component/src");
    let repo_ir = analyze(&root);

    assert!(
        repo_ir.files.iter().any(|f| f.file.ends_with("App.vue")),
        "App.vue should be indexed"
    );
    assert!(
        repo_ir
            .files
            .iter()
            .any(|f| f.file.ends_with("components/UserCard.vue")),
        "components/UserCard.vue should be indexed"
    );

    let app = repo_ir
        .files
        .iter()
        .find(|f| f.file.ends_with("App.vue"))
        .expect("App.vue exists");

    let svc_import = app.relationships.iter().find(|r| {
        r.kind == RelationshipKind::Imports
            && r.to.contains("UserService")
            && r.to.contains("user-service.ts")
    });
    assert!(
        svc_import.is_some(),
        "App.vue should resolve UserService import to user-service.ts, got: {:?}",
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
        "Vue template/style sections should be blanked without parser errors, got: {:?}",
        parse_errors
    );

    if let Some(imp) = svc_import {
        assert!(
            imp.line >= 7,
            "resolved import line should point to script section, got line {}",
            imp.line
        );
    }
}
