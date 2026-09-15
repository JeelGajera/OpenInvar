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
fn test_unresolved_local_type_does_not_bind_to_random_global_symbol() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/adapter-ts/collision/src");
    let repo_ir = analyze_repo(&root).expect("repo analysis must succeed");

    let usage = repo_ir
        .files
        .iter()
        .find(|f| f.file.ends_with("usage.ts"))
        .expect("usage file exists");

    let prop_rel = usage
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::AccessesProperty)
        .expect("property access relationship exists");

    // `usage.ts` names `Payload` without importing it, and two files under
    // models/ each declare one. The property being asserted is that the
    // reference binds to *neither*: picking one would be a coin flip presented
    // as a fact, and `blast-radius` would then point at a file the code never
    // mentions.
    //
    // It is still a placeholder at this layer because this calls the adapter
    // directly. `dispatch` drops every placeholder before the graph is built —
    // see `no_unresolved_reference_reaches_the_graph` — so what this pins is
    // the adapter refusing to guess, not an id that survives anywhere.
    assert!(
        openinvar_core::symbol_id::is_placeholder(&prop_rel.to),
        "an ambiguous type reference was bound to one of the candidates: {}",
        prop_rel.to
    );
    assert!(
        !prop_rel.to.contains("models/"),
        "the reference bound to a declaration in models/: {}",
        prop_rel.to
    );
    assert!(usage.diagnostics.iter().any(|d| d
        .message
        .contains("unable to resolve property-access type 'Payload'")));
}
