use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use openinvar_core::ir::{Language, RepoIR};
use rayon::prelude::*;

pub mod extractor;
pub mod framework;
pub mod import_resolver;
pub mod parser;
pub mod scope_analyzer;

#[derive(Debug)]
pub enum AdapterPythonError {
    Io(std::io::Error),
    Parse(String),
}

impl std::fmt::Display for AdapterPythonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::Parse(err) => write!(f, "parse error: {err}"),
        }
    }
}

impl std::error::Error for AdapterPythonError {}

impl From<std::io::Error> for AdapterPythonError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

pub fn analyze_files(root: &Path, files: &[PathBuf]) -> Result<RepoIR, AdapterPythonError> {
    let parse_results: Vec<Result<_, AdapterPythonError>> = files
        .par_iter()
        .map(|path| {
            let parsed = parser::parse_file(root, path).map_err(AdapterPythonError::Parse)?;
            Ok(extractor::extract_file_ir(&parsed))
        })
        .collect();

    let mut file_irs = Vec::with_capacity(files.len());
    let mut language_stats: BTreeMap<String, usize> = BTreeMap::new();

    for result in parse_results {
        let file_ir = result?;
        *language_stats
            .entry(format!("{:?}", file_ir.language))
            .or_insert(0) += 1;
        file_irs.push(file_ir);
    }

    let mut repo_ir = RepoIR {
        root: root.to_string_lossy().to_string(),
        files: file_irs,
        language_stats,
    };

    import_resolver::resolve_repo_ir(root, &mut repo_ir);

    Ok(repo_ir)
}

// ── language spec ────────────────────────────────────────────

/// This language's entry in the registry.
///
/// Tier 1: resolution here follows imports across files, tracks aliases, and
/// binds declared types for property attribution, so a gate may draw a
/// conclusion from it. The pipeline is [`analyze_files`] above, kept as it was
/// rather than rewritten into resolution hooks — it works and it is tested,
/// and a rewrite would forfeit the one property this change must preserve.
pub struct Spec;

impl crate::spec::LanguageSpec for Spec {
    /// `test_*.py` and `*_test.py`, plus `conftest.py`, which pytest collects
    /// as part of the test tree rather than as a module under test.
    fn is_test_file(&self, path: &str) -> bool {
        let name = path.rsplit('/').next().unwrap_or(path);
        let stem = name.strip_suffix(".py").unwrap_or(name);
        stem.starts_with("test_")
            || stem.ends_with("_test")
            || name == "conftest.py"
            || path.starts_with("tests/")
            || path.contains("/tests/")
    }

    fn language(&self) -> Language {
        Language::Python
    }

    fn tier(&self) -> crate::spec::Tier {
        crate::spec::Tier::Resolved
    }

    fn name(&self) -> &'static str {
        "Python"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["py", "pyi"]
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
