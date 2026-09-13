use std::collections::BTreeMap;
use std::path::Path;

use openinvar_core::ir::{Language, RepoIR};
use rayon::prelude::*;
pub mod extractor;
pub mod framework_preprocessor;
pub mod import_resolver;
pub mod language;
pub mod parser;
pub mod tsconfig;

#[derive(Debug)]
pub enum AdapterTsError {
    Io(std::io::Error),
    Parse(String),
}

impl std::fmt::Display for AdapterTsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::Parse(err) => write!(f, "parse error: {err}"),
        }
    }
}

impl std::error::Error for AdapterTsError {}

impl From<std::io::Error> for AdapterTsError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

pub fn analyze_files(root: &Path, files: &[std::path::PathBuf]) -> Result<RepoIR, AdapterTsError> {
    let parse_results: Vec<Result<_, AdapterTsError>> = files
        .par_iter()
        .map(|path| {
            let parsed = parser::parse_file(root, path).map_err(AdapterTsError::Parse)?;
            let mut file_ir = extractor::extract_file_ir(&parsed);
            if file_ir.diagnostics.is_empty() {
                file_ir.diagnostics.extend(parsed.diagnostics);
            }
            Ok(file_ir)
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
    /// `*.test.*` and `*.spec.*`, plus the directories bundlers and Jest
    /// already treat as test roots.
    fn is_test_file(&self, path: &str) -> bool {
        let name = path.rsplit('/').next().unwrap_or(path);
        let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
        stem.ends_with(".test")
            || stem.ends_with(".spec")
            || stem.ends_with("_test")
            || stem.ends_with("_spec")
            || path.starts_with("__tests__/")
            || path.contains("/__tests__/")
    }

    fn language(&self) -> Language {
        Language::TypeScript
    }

    fn tier(&self) -> crate::spec::Tier {
        crate::spec::Tier::Resolved
    }

    fn name(&self) -> &'static str {
        "TypeScript"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &[
            "ts", "tsx", "mts", "cts", "vue", "svelte", "astro", "js", "jsx", "mjs", "cjs",
        ]
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
