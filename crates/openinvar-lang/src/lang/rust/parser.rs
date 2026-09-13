use std::path::{Path, PathBuf};

use openinvar_core::ast::{first_error_line, has_parse_error};
use openinvar_core::ir::{Diagnostic, DiagnosticCategory, DiagnosticLevel, Language};
use openinvar_core::scan::looks_generated;
use tree_sitter::Tree;

#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub file: String,
    pub source: String,
    pub tree: Tree,
    pub language: Language,
    pub diagnostics: Vec<Diagnostic>,
}

fn parse_source(source: &str) -> Result<Tree, String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::language())
        .map_err(|e| format!("failed to set Rust language: {e}"))?;
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

    // Generated code is faithfully parseable but its symbols are noise in an
    // impact graph: nobody edits it, and it is regenerated wholesale.
    if looks_generated(&source, path) {
        return Ok(ParsedFile {
            file: file.clone(),
            source: String::new(),
            tree: parse_source("")?,
            language: Language::Rust,
            diagnostics: vec![Diagnostic {
                level: DiagnosticLevel::Info,
                category: DiagnosticCategory::Skip,
                message: "skipped generated file".to_string(),
                file: Some(file),
                line: None,
            }],
        });
    }

    let tree = parse_source(&source)?;
    let mut diagnostics = Vec::new();

    // tree-sitter always returns a tree; unparseable syntax becomes ERROR nodes.
    // Extraction still runs — a file with one bad function should contribute the
    // rest of its symbols — but the user is told the result is partial.
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
        source,
        tree,
        language: Language::Rust,
        diagnostics,
    })
}
