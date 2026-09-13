// Exercises the c module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "c")]

use std::path::{Path, PathBuf};

use openinvar_core::ir::RelationshipKind;
use openinvar_lang::lang::c::analyze_files;

fn fixture_root(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/adapter-c/{name}"))
}

fn all_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(root).into_iter().flatten() {
        if e.path().is_file()
            && matches!(
                e.path().extension().and_then(|x| x.to_str()),
                Some("c" | "h" | "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "hh")
            )
        {
            out.push(e.path().to_path_buf());
        }
    }
    out
}

#[test]
fn test_c_and_cpp_alias_import_is_caught() {
    for fix in ["alias_import_bug_c", "alias_import_bug_cpp"] {
        let root = fixture_root(fix);
        let repo_ir = analyze_files(&root, &all_files(&root)).expect("analysis must succeed");
        let mapper = repo_ir
            .files
            .iter()
            .find(|f| f.file.contains("view_model_mapper"))
            .expect("mapper exists");
        let rel = mapper
            .relationships
            .iter()
            .find(|r| {
                r.kind == RelationshipKind::Imports && r.alias.as_deref() == Some("ResponseModel")
            })
            .expect("aliased import exists");
        assert!(rel.to.contains("UserPayload"));
        assert!(!rel.properties_accessed.is_empty());
    }
}
