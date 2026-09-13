use std::path::{Path, PathBuf};

use openinvar_core::ast::{first_error_line, has_parse_error, walk};
use openinvar_core::ir::{Diagnostic, DiagnosticCategory, DiagnosticLevel, Language};
use openinvar_core::scan::looks_generated;
use tree_sitter::Tree;

#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub file: String,
    pub source: String,
    pub tree: Tree,
    pub language: Language,
    pub is_cpp: bool,
    pub diagnostics: Vec<Diagnostic>,
}

/// Extensions that are unambiguously C++.
const CPP_EXTENSIONS: &[&str] = &["cpp", "cc", "cxx", "c++", "hpp", "hxx", "hh", "h++", "tpp"];

/// Syntax that cannot appear in C, used to classify `.h` files.
const CPP_MARKERS: &[&str] = &[
    "class ",
    "namespace ",
    "template<",
    "template <",
    "public:",
    "private:",
    "protected:",
    "virtual ",
    "operator",
    "::",
    "std::",
    "nullptr",
    "constexpr",
    "#include <string>",
    "extern \"C\"",
];

fn grammar(is_cpp: bool) -> tree_sitter::Language {
    if is_cpp {
        tree_sitter_cpp::language()
    } else {
        tree_sitter_c::language()
    }
}

fn parse_with(source: &str, is_cpp: bool) -> Result<Tree, String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&grammar(is_cpp))
        .map_err(|e| format!("failed to set language: {e}"))?;
    parser
        .parse(source, None)
        .ok_or_else(|| "tree-sitter returned no parse tree".to_string())
}

/// Number of nodes tree-sitter could not parse.
fn error_count(tree: &Tree) -> usize {
    if !tree.root_node().has_error() {
        return 0;
    }
    let mut count = 0usize;
    walk(tree.root_node(), &mut |node| {
        if node.is_error() || node.is_missing() {
            count += 1;
        }
    });
    count
}

pub fn parse_file(root: &Path, path: &PathBuf) -> Result<ParsedFile, String> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| format!("failed reading {}: {e}", path.display()))?;
    let file = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if looks_generated(&source, path) {
        let is_cpp = CPP_EXTENSIONS.contains(&ext.as_str());
        return Ok(ParsedFile {
            file: file.clone(),
            source: String::new(),
            tree: parse_with("", is_cpp)?,
            language: if is_cpp { Language::Cpp } else { Language::C },
            is_cpp,
            diagnostics: vec![Diagnostic {
                level: DiagnosticLevel::Info,
                category: DiagnosticCategory::Skip,
                message: "skipped generated file".to_string(),
                file: Some(file),
                line: None,
            }],
        });
    }

    let (tree, is_cpp) = if ext == "h" {
        // `.h` is used by both languages. Parsing a C++ header with the C
        // grammar silently loses every class, namespace and template to error
        // recovery, so the grammar is chosen by what the file contains and
        // confirmed by which parse actually succeeds.
        choose_grammar_for_ambiguous_header(&source)?
    } else {
        let is_cpp = CPP_EXTENSIONS.contains(&ext.as_str());
        (parse_with(&source, is_cpp)?, is_cpp)
    };

    let mut diagnostics = Vec::new();
    if has_parse_error(tree.root_node()) {
        diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Error,
            category: DiagnosticCategory::Parse,
            message: format!(
                "syntax error while parsing as {}; symbols in the affected region \
                 were not extracted",
                if is_cpp { "C++" } else { "C" }
            ),
            file: Some(file.clone()),
            line: first_error_line(tree.root_node()),
        });
    }

    Ok(ParsedFile {
        file,
        source,
        tree,
        language: if is_cpp { Language::Cpp } else { Language::C },
        is_cpp,
        diagnostics,
    })
}

/// Pick C or C++ for a `.h` file, preferring whichever parses it cleanly.
fn choose_grammar_for_ambiguous_header(source: &str) -> Result<(Tree, bool), String> {
    let looks_cpp = CPP_MARKERS.iter().any(|marker| source.contains(marker));

    let first = parse_with(source, looks_cpp)?;
    let first_errors = error_count(&first);
    if first_errors == 0 {
        return Ok((first, looks_cpp));
    }

    // The heuristic and the grammar disagree; trust the parse.
    let second = parse_with(source, !looks_cpp)?;
    if error_count(&second) < first_errors {
        Ok((second, !looks_cpp))
    } else {
        Ok((first, looks_cpp))
    }
}
