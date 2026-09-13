//! The machine-readable shape of an analysis.
//!
//! OpenInvar's output is consumed by agents, hooks and CI as much as by people,
//! and prose is a poor contract for any of them. This module defines that
//! contract explicitly rather than letting it emerge from whatever `RepoIR`
//! happens to derive.
//!
//! Two properties matter and both are load-bearing:
//!
//! **It is versioned.** A consumer that pins `schema_version` can tell a shape
//! it understands from one it does not, instead of failing on a missing field
//! at the point of use. The version is here from the first release that emits
//! JSON at all, because retrofitting one means every existing consumer has to
//! handle its absence.
//!
//! **It is deterministic.** Identical input produces byte-identical output.
//! That is OpenInvar's defining property, and it only holds end to end if every
//! collection on the way out has a stable ordering key — see
//! [`openinvar_core::ir::RepoIR::language_stats`] for the case that did not.

use serde::Serialize;

use openinvar_core::ir::{FileIR, RepoIR};

/// The version of the JSON contract emitted by this build.
///
/// Bump on any change a consumer could observe: a removed or renamed field, a
/// narrowed type, or a change in the meaning of an existing field. Adding an
/// optional field is not a bump — consumers are expected to ignore unknown
/// keys.
pub const SCHEMA_VERSION: u32 = 1;

/// One analysis, as emitted by `openinvar analyze --json`.
#[derive(Debug, Serialize)]
pub struct AnalysisReport<'a> {
    pub schema_version: u32,
    /// Absolute path of the analyzed repository root.
    ///
    /// The only absolute path in the report: every path inside `files` is
    /// relative to it, which is what lets two checkouts of the same revision
    /// in different directories produce identical `files`.
    pub root: &'a str,
    pub stats: ReportStats,
    pub language_stats: &'a std::collections::BTreeMap<String, usize>,
    pub files: &'a [FileIR],
}

/// Headline counts, matching what the human summary prints.
#[derive(Debug, Serialize)]
pub struct ReportStats {
    pub symbols: usize,
    pub relationships: usize,
    pub files_indexed: usize,
    /// Symbols that have at least one alias.
    pub alias_chains: usize,
    /// Aliases in total. Added alongside `alias_chains` rather than replacing
    /// it: adding a field is not a contract change, redefining one is.
    pub aliases: usize,
    pub diagnostics: usize,
}

impl<'a> AnalysisReport<'a> {
    pub fn new(repo_ir: &'a RepoIR, stats: &super::analyze::AnalyzeStats) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            root: &repo_ir.root,
            stats: ReportStats {
                symbols: stats.symbols,
                relationships: stats.relationships,
                files_indexed: repo_ir.files.len(),
                alias_chains: stats.alias_chains,
                aliases: stats.aliases,
                diagnostics: repo_ir.files.iter().map(|f| f.diagnostics.len()).sum(),
            },
            language_stats: &repo_ir.language_stats,
            files: &repo_ir.files,
        }
    }

    /// Render as pretty-printed JSON.
    ///
    /// Pretty rather than compact because the primary consumer of a stored
    /// report is a line-based diff — of two revisions, or of a golden fixture
    /// against a fresh run. One field per line makes that diff readable; a
    /// single 40 MB line does not.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}
