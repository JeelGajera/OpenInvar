//! C# — Tier 1.
//!
//! This shipped as a Tier 2 language, with its module saying plainly that
//! nothing in it resolved an import, followed an alias, or bound a declared
//! type. All three are here now, and gates may act on C#.
//!
//! # What made this harder than Java
//!
//! **`using` names a namespace, not a type.** C# has no single-type import, so
//! every `using` is the on-demand form and the ambiguity Java only reaches
//! through wildcards is C#'s ordinary case.
//!
//! **Enclosing namespaces are in scope.** Inside `namespace A.B.C`, a type in
//! `A.B` or `A` resolves with no `using` at all, innermost first.
//!
//! **A type can span files.** `partial class Report` in two files is one type
//! with the union of its members, so a call to the other half's method has to
//! resolve. Members are indexed by the owner's fully-qualified name rather than
//! by declaration, which makes that fall out instead of needing a special case.
//!
//! `var` is deliberately not inferred — see [`scope_analyzer`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use openinvar_core::ir::{Language, RepoIR};
use rayon::prelude::*;

pub mod extractor;
pub mod import_resolver;
pub mod parser;
pub mod scope_analyzer;

#[derive(Debug)]
pub enum AdapterCSharpError {
    Parse(String),
}

impl std::fmt::Display for AdapterCSharpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(err) => write!(f, "parse error: {err}"),
        }
    }
}
impl std::error::Error for AdapterCSharpError {}

pub fn analyze_files(root: &Path, files: &[PathBuf]) -> Result<RepoIR, AdapterCSharpError> {
    let parsed: Vec<Result<_, AdapterCSharpError>> = files
        .par_iter()
        .map(|path| {
            let parsed = parser::parse_file(root, path).map_err(AdapterCSharpError::Parse)?;
            let (mut file_ir, facts) = extractor::extract(&parsed);
            file_ir.relationships.extend(scope_analyzer::analyze(&parsed));
            Ok((file_ir, facts))
        })
        .collect();

    let mut file_irs = Vec::with_capacity(files.len());
    let mut all_facts = Vec::with_capacity(files.len());
    let mut language_stats: BTreeMap<String, usize> = BTreeMap::new();

    for result in parsed {
        let (file_ir, facts) = result?;
        *language_stats
            .entry(format!("{:?}", file_ir.language))
            .or_insert(0) += 1;
        file_irs.push(file_ir);
        all_facts.push(facts);
    }

    // Cross-file resolution needs every declaration — and for a partial type,
    // every half of it — so it runs last and over everything at once.
    import_resolver::resolve(&mut file_irs, &all_facts);

    Ok(RepoIR {
        root: root.to_string_lossy().to_string(),
        files: file_irs,
        language_stats,
    })
}

// ── language spec ────────────────────────────────────────────

pub struct Spec;

impl crate::spec::LanguageSpec for Spec {
    /// The convention shared by xUnit, NUnit and MSTest projects.
    fn is_test_file(&self, path: &str) -> bool {
        let name = path.rsplit('/').next().unwrap_or(path);
        let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
        stem.ends_with("Test")
            || stem.ends_with("Tests")
            || stem.ends_with("Spec")
            || path.contains(".Tests/")
            || path.contains("/Tests/")
    }

    fn language(&self) -> Language {
        Language::CSharp
    }

    fn tier(&self) -> crate::spec::Tier {
        crate::spec::Tier::Resolved
    }

    fn name(&self) -> &'static str {
        "C#"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["cs"]
    }

    fn grammar(&self) -> Option<tree_sitter::Language> {
        Some(tree_sitter_c_sharp::LANGUAGE.into())
    }

    fn analyze(
        &self,
        root: &std::path::Path,
        files: &[std::path::PathBuf],
    ) -> Option<Result<Vec<openinvar_core::ir::FileIR>, String>> {
        Some(
            analyze_files(root, files)
                .map(|ir| ir.files)
                .map_err(|e| e.to_string()),
        )
    }
}
