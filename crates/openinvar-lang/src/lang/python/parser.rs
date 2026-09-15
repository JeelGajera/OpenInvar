use std::path::{Path, PathBuf};

use openinvar_core::ast::{first_error_line, has_parse_error};
use openinvar_core::ir::{Diagnostic, DiagnosticCategory, DiagnosticLevel, Language};
use openinvar_core::scan::looks_generated;
use tree_sitter::Tree;

#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub file: String,
    pub language: Language,
    pub source: String,
    pub tree: Tree,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn parse_python(source: &str) -> Result<Tree, String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .map_err(|e| format!("failed to set Python language: {e}"))?;
    parser
        .parse(source, None)
        .ok_or_else(|| "tree-sitter returned no parse tree".to_string())
}

pub fn parse_file(root: &Path, path: &PathBuf) -> Result<ParsedFile, String> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| format!("failed reading {}: {e}", path.display()))?;
    let file = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");

    if looks_generated(&source, path) {
        return Ok(ParsedFile {
            file: file.clone(),
            language: Language::Python,
            source: String::new(),
            tree: parse_python("")?,
            diagnostics: vec![Diagnostic {
                level: DiagnosticLevel::Info,
                category: DiagnosticCategory::Skip,
                message: "skipped generated file".to_string(),
                file: Some(file),
                line: None,
            }],
        });
    }

    let tree = parse_python(&source)?;
    let mut diagnostics = Vec::new();

    if has_parse_error(tree.root_node()) {
        diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Error,
            category: DiagnosticCategory::Parse,
            message: "syntax error; symbols in the affected region were not extracted".to_string(),
            file: Some(file.clone()),
            line: first_error_line(tree.root_node()),
        });
    }

    Ok(ParsedFile {
        file,
        language: Language::Python,
        source,
        tree,
        diagnostics,
    })
}
