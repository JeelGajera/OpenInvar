use std::path::Path;

use openinvar_core::ir::{Diagnostic, DiagnosticCategory, DiagnosticLevel, Language};
use tree_sitter::Tree;

use crate::lang::typescript::framework_preprocessor::extract_script_content;
use crate::lang::typescript::language::detect_language;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceDialect {
    TypeScript,
    Tsx,
    JavaScript,
    Jsx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameworkKind {
    Vue,
    Svelte,
    Astro,
    None,
}

pub struct ParsedFile {
    pub file: String,
    pub language: Language,
    pub dialect: SourceDialect,
    pub source: String,
    pub tree: Tree,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn parse_file(root: &Path, path: &Path) -> Result<ParsedFile, String> {
    let dialect = detect_dialect(path)
        .ok_or_else(|| format!("unsupported file extension: {}", path.display()))?;
    let language = detect_language(path)
        .ok_or_else(|| format!("unsupported file extension: {}", path.display()))?;
    let framework = detect_framework(path);

    let raw_source = std::fs::read_to_string(path)
        .map_err(|err| format!("failed reading {}: {err}", path.display()))?;

    let file = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");

    // Skip likely minified/bundled files — they produce garbage symbols
    if is_likely_minified(&raw_source) {
        let tree = parse_source(dialect, "")?;
        return Ok(ParsedFile {
            file,
            language,
            dialect,
            source: String::new(),
            tree,
            diagnostics: vec![Diagnostic {
                level: DiagnosticLevel::Info,
                category: DiagnosticCategory::Skip,
                message: "skipped likely minified file".to_string(),
                file: None,
                line: None,
            }],
        });
    }

    let source = if framework == FrameworkKind::None {
        raw_source
    } else {
        extract_script_content(&raw_source, framework)
    };

    let tree = parse_source(dialect, &source)?;
    let diagnostics = collect_parse_errors(tree.root_node(), &source);

    Ok(ParsedFile {
        file,
        language,
        dialect,
        source,
        tree,
        diagnostics,
    })
}

fn is_likely_minified(source: &str) -> bool {
    let total_bytes = source.len();
    if total_bytes < 1000 {
        return false; // small files are never minified
    }

    let line_count = source.lines().count().max(1);
    let avg_line_len = total_bytes / line_count;

    // Signal 1: very high average line length
    if avg_line_len > 200 {
        return true;
    }

    // Signal 2: extremely long single line
    if source.lines().any(|l| l.len() > 1000) {
        return true;
    }

    // Signal 3: low whitespace ratio in a large file
    if total_bytes > 5000 {
        let whitespace = source.bytes().filter(|b| b.is_ascii_whitespace()).count();
        let ratio = whitespace as f64 / total_bytes as f64;
        if ratio < 0.05 {
            return true;
        }
    }

    false
}

fn parse_source(dialect: SourceDialect, source: &str) -> Result<Tree, String> {
    let mut parser = tree_sitter::Parser::new();
    let ts_language = match dialect {
        SourceDialect::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        SourceDialect::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        // tree-sitter-javascript handles both JS and JSX for this crate version.
        SourceDialect::JavaScript | SourceDialect::Jsx => tree_sitter_javascript::LANGUAGE.into(),
    };

    parser
        .set_language(&ts_language)
        .map_err(|err| format!("failed to set parser language: {err}"))?;
    parser
        .parse(source, None)
        .ok_or_else(|| "tree-sitter returned no parse tree".to_string())
}

fn detect_dialect(path: &Path) -> Option<SourceDialect> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    match ext.as_str() {
        "ts" | "mts" | "cts" => Some(SourceDialect::TypeScript),
        "tsx" | "vue" | "svelte" | "astro" => Some(SourceDialect::Tsx),
        "js" | "mjs" | "cjs" => Some(SourceDialect::JavaScript),
        "jsx" => Some(SourceDialect::Jsx),
        _ => None,
    }
}

/// Detect framework-like file kinds that require script extraction.
pub fn detect_framework(path: &Path) -> FrameworkKind {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("vue") => FrameworkKind::Vue,
        Some("svelte") => FrameworkKind::Svelte,
        Some("astro") => FrameworkKind::Astro,
        _ => FrameworkKind::None,
    }
}

fn collect_parse_errors(node: tree_sitter::Node<'_>, source: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    collect_parse_errors_recursive(node, source, &mut out);
    out.sort_by(|a, b| a.message.cmp(&b.message));
    out.dedup_by(|a, b| a.message == b.message);
    out
}

fn collect_parse_errors_recursive(
    node: tree_sitter::Node<'_>,
    source: &str,
    out: &mut Vec<Diagnostic>,
) {
    if node.is_error() {
        let row = node.start_position().row + 1;
        let line = source
            .lines()
            .nth(row.saturating_sub(1))
            .unwrap_or_default();
        out.push(Diagnostic {
            level: DiagnosticLevel::Error,
            category: DiagnosticCategory::Parse,
            message: format!("line {row}: {}", line.trim()),
            file: None,
            line: Some(row as u32),
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_parse_errors_recursive(child, source, out);
    }
}
