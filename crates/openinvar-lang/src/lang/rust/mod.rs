use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use openinvar_core::ir::{Language, RepoIR};
use rayon::prelude::*;

pub mod crate_set;
pub mod extractor;
pub mod import_resolver;
pub mod macro_analyzer;
pub mod module_tree;
pub mod parser;
pub mod scope_analyzer;

#[derive(Debug)]
pub enum AdapterRustError {
    Io(std::io::Error),
    Parse(String),
}

impl std::fmt::Display for AdapterRustError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::Parse(err) => write!(f, "parse error: {err}"),
        }
    }
}

impl std::error::Error for AdapterRustError {}

pub fn analyze_files(root: &Path, files: &[PathBuf]) -> Result<RepoIR, AdapterRustError> {
    let parse_results: Vec<Result<_, AdapterRustError>> = files
        .par_iter()
        .map(|path| {
            let parsed = parser::parse_file(root, path).map_err(AdapterRustError::Parse)?;
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
    /// A crate's `tests/` directory, plus `*_test.rs` and `*_tests.rs`.
    ///
    /// Not `#[cfg(test)] mod tests` inside an ordinary source file, which is
    /// the more common Rust convention and the one this repository uses most.
    /// Those symbols are not indexed at all today — see
    /// [`crate::spec::LanguageSpec::is_test_file`] — so recognising the module
    /// here would name a test whose contents OpenInvar cannot see.
    fn is_test_file(&self, path: &str) -> bool {
        let name = path.rsplit('/').next().unwrap_or(path);
        let stem = name.strip_suffix(".rs").unwrap_or(name);
        stem.ends_with("_test")
            || stem.ends_with("_tests")
            || path.starts_with("tests/")
            || path.contains("/tests/")
            || path.starts_with("benches/")
            || path.contains("/benches/")
    }

    fn language(&self) -> Language {
        Language::Rust
    }

    fn tier(&self) -> crate::spec::Tier {
        crate::spec::Tier::Resolved
    }

    fn name(&self) -> &'static str {
        "Rust"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["rs"]
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
