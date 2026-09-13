use std::path::{Path, PathBuf};

fn all_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(root).into_iter().flatten() {
        if e.path().is_file() {
            out.push(e.path().to_path_buf());
        }
    }
    out
}

#[test]
fn test_polyglot_repo_is_analyzed_without_errors() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/polyglot");
    let repo_ir = openinvar_lang::analyze_files(&root, &all_files(&root))
        .expect("polyglot analysis must succeed");

    assert!(repo_ir.language_stats.contains_key("TypeScript"));
    assert!(repo_ir.language_stats.contains_key("Python"));
    assert!(repo_ir.language_stats.contains_key("Rust"));
    assert!(repo_ir.language_stats.contains_key("Go"));
}
