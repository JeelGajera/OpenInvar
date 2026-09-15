use std::path::{Path, PathBuf};

use openinvar_core::ast::{first_error_line, has_parse_error, node_text};
use openinvar_core::ir::{Diagnostic, DiagnosticCategory, DiagnosticLevel, Language};
use openinvar_core::scan::looks_generated;
use tree_sitter::Tree;

#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub file: String,
    pub source: String,
    pub tree: Tree,
    pub language: Language,
    /// The `package` clause, which names the file's namespace.
    pub package_name: String,
    pub diagnostics: Vec<Diagnostic>,
}

fn parse_source(source: &str) -> Result<Tree, String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_go::LANGUAGE.into())
        .map_err(|e| format!("failed to set Go language: {e}"))?;
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
            source: String::new(),
            tree: parse_source("")?,
            language: Language::Go,
            package_name: String::new(),
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

    if has_parse_error(tree.root_node()) {
        diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Error,
            category: DiagnosticCategory::Parse,
            message: "syntax error; symbols in the affected region were not extracted".to_string(),
            file: Some(file.clone()),
            line: first_error_line(tree.root_node()),
        });
    }

    let package_name = package_clause(&tree, source.as_bytes()).unwrap_or_default();

    Ok(ParsedFile {
        file,
        source,
        tree,
        language: Language::Go,
        package_name,
        diagnostics,
    })
}

/// Read the `package x` declaration, which is always the first clause.
fn package_clause(tree: &Tree, source: &[u8]) -> Option<String> {
    let root = tree.root_node();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() != "package_clause" {
            continue;
        }
        let mut inner = child.walk();
        for node in child.children(&mut inner) {
            if matches!(node.kind(), "package_identifier" | "identifier") {
                return node_text(node, source).map(str::to_string);
            }
        }
    }
    None
}
