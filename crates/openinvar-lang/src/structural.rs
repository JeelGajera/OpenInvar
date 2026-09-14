//! Tier 2 analysis: one implementation, every language.
//!
//! Driven entirely by the `tags.scm` a tree-sitter grammar already ships and
//! exposes as `TAGS_QUERY`. Nothing here is language-specific, because the
//! capture vocabulary those files use is not:
//!
//! ```text
//! @definition.class  @definition.function  @definition.method
//! @definition.type   @definition.interface @definition.module
//! @definition.constant @definition.macro
//! @reference.call    @reference.type       @reference.class
//! @reference.implementation
//! @name
//! ```
//!
//! That vocabulary maps onto OpenInvar's own `SymbolKind` and
//! `RelationshipKind`, which is what makes a generic implementation possible
//! and why adding a Tier 2 language needs no query authoring at all.
//!
//! # What this deliberately does not do
//!
//! **No cross-file resolution.** A reference is recorded only when the name it
//! names is defined in the same file. `tags.scm` reports that a call to `foo`
//! happened; it does not say which `foo`, and guessing repository-wide is
//! exactly the bug 0.2.0 fixed in the Rust adapter, where use-paths matched
//! leaf names anywhere in the tree.
//!
//! **No aliases and no declared types.** Both require import resolution to
//! mean anything.
//!
//! This is why Tier 2 is not gate-safe. "Nothing references this symbol" from
//! a Tier 2 file is a statement about one file, and a gate that read it as a
//! statement about the repository would report a pass it had not earned.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use openinvar_core::ir::{Diagnostic, DiagnosticCategory, DiagnosticLevel, FileIR, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind};
use openinvar_core::symbol_id::{make_symbol_id, module_symbol, module_symbol_id};
use rayon::prelude::*;
use tree_sitter::{Parser, Query, QueryCursor};

use crate::spec::LanguageSpec;

/// Map a `@definition.*` capture to a symbol kind.
///
/// An unrecognised suffix is skipped rather than guessed at: inventing a kind
/// for a capture nobody has mapped would put a symbol of the wrong kind into
/// the graph, and a wrong answer is worse than a missing one.
fn definition_kind(suffix: &str) -> Option<SymbolKind> {
    Some(match suffix {
        "class" => SymbolKind::Class,
        "interface" => SymbolKind::Interface,
        "function" => SymbolKind::Function,
        "method" => SymbolKind::Method,
        "type" => SymbolKind::TypeAlias,
        "module" => SymbolKind::Module,
        "constant" => SymbolKind::Variable,
        "enum" => SymbolKind::Enum,
        // `macro` and anything a future grammar introduces.
        _ => return None,
    })
}

/// Map a `@reference.*` capture to a relationship kind.
fn reference_kind(suffix: &str) -> Option<RelationshipKind> {
    Some(match suffix {
        "call" => RelationshipKind::Calls,
        "type" | "class" => RelationshipKind::UsesType,
        "implementation" => RelationshipKind::Implements,
        _ => return None,
    })
}

/// Analyse `files` structurally, in parallel.
pub fn analyze(
    spec: &dyn LanguageSpec,
    root: &Path,
    files: &[PathBuf],
) -> Result<Vec<FileIR>, String> {
    let grammar = spec
        .grammar()
        .ok_or_else(|| format!("{} has no grammar", spec.name()))?;
    let tags = spec
        .tags_query()
        .ok_or_else(|| format!("{} has no tags query", spec.name()))?;

    // Compiled once and shared: a tags query is a few hundred nodes and
    // recompiling it per file dominated the parse on a large tree.
    let query = Query::new(&grammar, tags)
        .map_err(|e| format!("{}: invalid tags query: {e}", spec.name()))?;

    files
        .par_iter()
        .map(|path| analyze_file(spec, &grammar, &query, root, path))
        .collect()
}

fn analyze_file(
    spec: &dyn LanguageSpec,
    grammar: &tree_sitter::Language,
    query: &Query,
    root: &Path,
    path: &Path,
) -> Result<FileIR, String> {
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");

    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            // An unreadable file is reported rather than dropped: a file that
            // vanishes from the graph with no diagnostic reads as a parser bug.
            return Ok(empty_with_diagnostic(
                spec,
                &relative,
                DiagnosticLevel::Error,
                DiagnosticCategory::Parse,
                format!("cannot read file: {e}"),
            ));
        }
    };

    let mut parser = Parser::new();
    parser
        .set_language(grammar)
        .map_err(|e| format!("{}: cannot load grammar: {e}", spec.name()))?;
    let Some(tree) = parser.parse(&source, None) else {
        return Ok(empty_with_diagnostic(
            spec,
            &relative,
            DiagnosticLevel::Error,
            DiagnosticCategory::Parse,
            "tree-sitter returned no tree".to_string(),
        ));
    };

    let mut symbols = vec![module_symbol(&relative, spec.language())];
    // Keyed by name so a reference can be matched against a definition in the
    // same file. Ordered, because it decides which symbol a repeated name
    // resolves to and that must not vary between runs.
    let mut defined: BTreeMap<String, String> = BTreeMap::new();
    let mut references: Vec<(String, RelationshipKind, u32)> = Vec::new();

    let mut cursor = QueryCursor::new();
    let names = query.capture_names();
    let matches = cursor.matches(query, tree.root_node(), source.as_bytes());

    for m in matches {
        // A tags.scm pattern pairs one `@name` with one `@definition.*` or
        // `@reference.*` in the same match.
        let mut name: Option<(&str, u32, u32)> = None;
        let mut role: Option<(&str, u32, u32)> = None;

        for capture in m.captures {
            let capture_name = names[capture.index as usize];
            let text = capture.node.utf8_text(source.as_bytes()).unwrap_or("");
            let start = capture.node.start_position().row as u32 + 1;
            let end = capture.node.end_position().row as u32 + 1;

            if capture_name == "name" {
                name = Some((text, start, end));
            } else if let Some(suffix) = capture_name.strip_prefix("definition.") {
                role = Some((suffix, start, end));
            } else if let Some(suffix) = capture_name.strip_prefix("reference.") {
                role = Some((suffix, start, end));
            }
        }

        let (Some((symbol_name, name_line, _)), Some((suffix, start, end))) = (name, role) else {
            continue;
        };
        if symbol_name.is_empty() {
            continue;
        }

        let is_definition = names
            .iter()
            .any(|n| n.strip_prefix("definition.") == Some(suffix))
            && m.captures.iter().any(|c| {
                names[c.index as usize]
                    .strip_prefix("definition.")
                    .is_some()
            });

        if is_definition {
            let Some(kind) = definition_kind(suffix) else {
                continue;
            };
            let id = make_symbol_id(&relative, symbol_name, &kind);
            if defined.contains_key(symbol_name) {
                // First definition wins, and inputs are sorted upstream, so
                // which one that is stays stable across runs.
                continue;
            }
            defined.insert(symbol_name.to_string(), id.clone());
            symbols.push(Symbol {
                id,
                name: symbol_name.to_string(),
                kind,
                language: spec.language(),
                file: relative.clone(),
                line_start: start,
                line_end: end.max(start),
                signature: None,
            });
        } else if let Some(kind) = reference_kind(suffix) {
            references.push((symbol_name.to_string(), kind, name_line));
        }
    }

    // Only references whose target is defined in this same file become edges.
    // Matching a name against the whole repository is the bug 0.2.0 fixed;
    // doing it here under the banner of "broad language support" would
    // reintroduce it across thirty languages at once.
    let module = module_symbol_id(&relative);
    let mut relationships: Vec<Relationship> = references
        .into_iter()
        .filter_map(|(name, kind, line)| {
            let to = defined.get(&name)?;
            Some(Relationship {
                from: module.clone(),
                to: to.clone(),
                kind,
                alias: None,
                properties_accessed: Vec::new(),
                context: String::new(),
                file: relative.clone(),
                line,
                resolution: Resolution::default(),
            })
        })
        .collect();

    relationships.sort_by(|a, b| {
        a.line
            .cmp(&b.line)
            .then(a.to.cmp(&b.to))
            .then(format!("{:?}", a.kind).cmp(&format!("{:?}", b.kind)))
    });
    relationships.dedup_by(|a, b| a.to == b.to && a.line == b.line && a.kind == b.kind);

    Ok(FileIR {
        file: relative,
        language: spec.language(),
        symbols,
        relationships,
        diagnostics: Vec::new(),
        re_exports: Vec::new(),
        // Structural analysis resolves nothing and counts nothing. The
        // empty map is read as "unknown" because counts_assertions() is
        // false for every Tier 2 language.
        assertions: Default::default(),
        assertions_counted: false,
    })
}

fn empty_with_diagnostic(
    spec: &dyn LanguageSpec,
    relative: &str,
    level: DiagnosticLevel,
    category: DiagnosticCategory,
    message: String,
) -> FileIR {
    FileIR {
        file: relative.to_string(),
        language: spec.language(),
        symbols: Vec::new(),
        relationships: Vec::new(),
        diagnostics: vec![Diagnostic {
            level,
            category,
            message,
            file: Some(relative.to_string()),
            line: None,
        }],
        re_exports: Vec::new(),
        // Structural analysis resolves nothing and counts nothing. The
        // empty map is read as "unknown" because counts_assertions() is
        // false for every Tier 2 language.
        assertions: Default::default(),
        assertions_counted: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_suffixes_map_to_kinds() {
        assert_eq!(definition_kind("class"), Some(SymbolKind::Class));
        assert_eq!(definition_kind("method"), Some(SymbolKind::Method));
        assert_eq!(reference_kind("call"), Some(RelationshipKind::Calls));
        assert_eq!(
            reference_kind("implementation"),
            Some(RelationshipKind::Implements)
        );
    }

    #[test]
    fn an_unknown_capture_is_skipped_rather_than_guessed() {
        // A grammar introducing `@definition.trait` should produce no symbol
        // rather than a symbol of an invented kind. A missing symbol is a gap;
        // a wrongly-kinded one is a wrong answer.
        assert_eq!(definition_kind("trait"), None);
        assert_eq!(definition_kind("macro"), None);
        assert_eq!(reference_kind("something-new"), None);
    }
}
