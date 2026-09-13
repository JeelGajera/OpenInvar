// Exercises the go module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "go")]

use std::path::{Path, PathBuf};

use openinvar_core::ir::RelationshipKind;
use openinvar_lang::lang::go::analyze_files;

fn fixture_root(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/adapter-go/{name}"))
}

fn all_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(root).into_iter().flatten() {
        if e.path().is_file()
            && matches!(
                e.path().extension().and_then(|x| x.to_str()),
                Some("go" | "mod")
            )
        {
            out.push(e.path().to_path_buf());
        }
    }
    out
}

#[test]
fn test_go_alias_import_is_caught_with_property_access() {
    let root = fixture_root("alias_import_bug");
    let repo_ir = analyze_files(&root, &all_files(&root)).expect("analysis must succeed");
    let mapper = repo_ir
        .files
        .iter()
        .find(|f| f.file.ends_with("view_model_mapper.go"))
        .expect("mapper exists");
    let rel = mapper
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::Imports && r.alias.as_deref() == Some("models"))
        .expect("aliased import exists");
    assert!(
        rel.to.contains("UserPayload") || rel.to.contains("models"),
        "import should resolve to UserPayload or the models package, got: {}",
        rel.to
    );
    assert!(rel.properties_accessed.contains(&"UserID".to_string()));
}
