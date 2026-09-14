//! Java — Tier 1.
//!
//! This was the first Tier 2 language, and its module said so plainly:
//!
//! > Promoting Java to Tier 1 means implementing import resolution, alias
//! > resolution and declared-type binding for it.
//!
//! That is what is here. Java turns out to be the most tractable language in
//! the set for the job, because everything resolution needs is written down:
//! every variable declares its type, every import is explicit, `package`
//! states where a file's names live, and there is no autoloading or runtime
//! monkey-patching to make a reference undecidable.
//!
//! The pipeline is the same shape as the other Tier 1 adapters — parse,
//! extract, resolve — with the resolution split into two passes because the
//! two questions are different. [`scope_analyzer`] answers *what type is this
//! receiver*, which is a within-file question. [`import_resolver`] answers
//! *which declaration does this simple name refer to*, which needs every file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use openinvar_core::ir::{Language, RepoIR};
use rayon::prelude::*;

pub mod extractor;
pub mod import_resolver;
pub mod parser;
pub mod scope_analyzer;

#[derive(Debug)]
pub enum AdapterJavaError {
    Parse(String),
}

impl std::fmt::Display for AdapterJavaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(err) => write!(f, "parse error: {err}"),
        }
    }
}
impl std::error::Error for AdapterJavaError {}

pub fn analyze_files(root: &Path, files: &[PathBuf]) -> Result<RepoIR, AdapterJavaError> {
    let parsed: Vec<Result<_, AdapterJavaError>> = files
        .par_iter()
        .map(|path| {
            let parsed = parser::parse_file(root, path).map_err(AdapterJavaError::Parse)?;
            let (mut file_ir, facts) = extractor::extract(&parsed);
            // Receivers are bound here, while the tree is still in hand.
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

    // Cross-file resolution needs every declaration, so it runs last and over
    // everything at once.
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
    /// Maven and Gradle both put tests under `src/test/`, and JUnit classes
    /// conventionally end in `Test` or `Tests`.
    fn is_test_file(&self, path: &str) -> bool {
        let name = path.rsplit('/').next().unwrap_or(path);
        let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
        stem.ends_with("Test")
            || stem.ends_with("Tests")
            || stem.starts_with("Test")
            || path.contains("src/test/")
    }

    fn language(&self) -> Language {
        Language::Java
    }

    fn tier(&self) -> crate::spec::Tier {
        crate::spec::Tier::Resolved
    }

    fn name(&self) -> &'static str {
        "Java"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["java"]
    }

    fn grammar(&self) -> Option<tree_sitter::Language> {
        Some(tree_sitter_java::language())
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
