use std::path::Path;

use schemars::JsonSchema;
use serde::Deserialize;

use openinvar_core::graph::InvarGraph;
use openinvar_core::ir::RepoIR;
use openinvar_core::resolver::AliasResolver;
use openinvar_core::scan::{
    is_any_supported_source_file, parse_csv_patterns, walk_source_files_with_config, ScanConfig,
};
use openinvar_lang::analyze_files;
use openinvar_store::RocksGraphStore;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RefreshGraphParams {
    /// Optional relative root path under server repo root.
    pub path: Option<String>,
    /// Comma-separated include patterns.
    pub include: Option<String>,
    /// Comma-separated exclude patterns.
    pub exclude: Option<String>,
    /// Respect .gitignore rules. Default true.
    pub respect_gitignore: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct RefreshResult {
    pub symbols: usize,
    pub relationships: usize,
    pub alias_chains: usize,
    pub files_indexed: usize,
    pub diagnostics: usize,
}

pub fn execute(
    repo_root: &Path,
    params: RefreshGraphParams,
) -> Result<(InvarGraph, RefreshResult), String> {
    let analysis_root = match params.path.as_deref() {
        Some(rel) if !rel.trim().is_empty() => repo_root.join(rel),
        _ => repo_root.to_path_buf(),
    };

    let root = std::fs::canonicalize(&analysis_root)
        .map_err(|e| format!("cannot access '{}': {e}", analysis_root.display()))?;

    let scan_config = ScanConfig {
        include_patterns: parse_csv_patterns(params.include.as_deref()),
        exclude_patterns: parse_csv_patterns(params.exclude.as_deref()),
        respect_gitignore: params.respect_gitignore.unwrap_or(true),
    };

    let files = walk_source_files_with_config(&root, &scan_config, is_any_supported_source_file)
        .map_err(|e| format!("scan failed: {e}"))?;

    let repo_ir = analyze_files(&root, &files).map_err(|e| format!("analysis failed: {e}"))?;

    let diagnostics: usize = repo_ir.files.iter().map(|f| f.diagnostics.len()).sum();
    let graph = build_graph(&repo_ir);

    let db = root.join(".openinvar").join("db");
    if let Some(parent) = db.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("failed to create db dir: {e}"))?;
    }
    let store = RocksGraphStore::open(&db).map_err(|e| format!("failed to open store: {e}"))?;
    store
        .save_graph(&graph)
        .map_err(|e| format!("failed to save graph: {e}"))?;

    let result = RefreshResult {
        symbols: graph.symbols.len(),
        relationships: graph.graph.edge_count(),
        alias_chains: graph.alias_count(),
        files_indexed: repo_ir.files.len(),
        diagnostics,
    };

    Ok((graph, result))
}

fn build_graph(repo_ir: &RepoIR) -> InvarGraph {
    let mut graph = InvarGraph::new();
    let resolver = AliasResolver::default();

    for file_ir in &repo_ir.files {
        for symbol in &file_ir.symbols {
            graph.add_symbol(symbol.clone());
        }
    }

    for file_ir in &repo_ir.files {
        for relationship in &file_ir.relationships {
            graph.add_relationship(relationship);
        }
        graph
            .file_reexports
            .insert(file_ir.file.clone(), file_ir.re_exports.clone());
        resolver.ingest_relationships(&graph, &file_ir.relationships);
    }

    graph
}
