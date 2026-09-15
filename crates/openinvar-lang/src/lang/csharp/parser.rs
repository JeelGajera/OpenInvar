use std::path::Path;

use openinvar_core::ir::{Diagnostic, DiagnosticCategory, DiagnosticLevel, Language};
use tree_sitter::Tree;

pub struct ParsedFile {
    pub file: String,
    pub language: Language,
    pub source: String,
    pub tree: Tree,
    pub diagnostics: Vec<Diagnostic>,
}

fn parse_source(source: &str) -> Result<Tree, String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_c_sharp::language())
        .map_err(|e| format!("failed to set C# language: {e}"))?;
    parser
        .parse(source, None)
        .ok_or_else(|| "tree-sitter returned no tree".to_string())
}

pub fn parse_file(root: &Path, path: &Path) -> Result<ParsedFile, String> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let tree = parse_source(&source)?;

    let file = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");

    // Recorded rather than fatal: one unparseable file should not cost the
    // analysis of every other file, and the diagnostic is what tells a reader
    // the region is incomplete.
    let mut diagnostics = Vec::new();
    if tree.root_node().has_error() {
        diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Error,
            category: DiagnosticCategory::Parse,
            message: "the file did not parse cleanly; symbols may be missing".to_string(),
            file: Some(file.clone()),
            line: None,
        });
    }

    Ok(ParsedFile {
        file,
        language: Language::CSharp,
        source,
        tree,
        diagnostics,
    })
}
