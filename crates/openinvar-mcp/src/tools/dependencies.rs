//! MCP tool: get_dependencies
//!
//! Returns everything a given symbol depends on — its full dependency tree.

use schemars::JsonSchema;
use serde::Deserialize;

use openinvar_core::graph::InvarGraph;
use openinvar_core::query;

use crate::context_builder;
use crate::tools::kinds::{
    absent_kinds_warning, mask_from_names, resolution_from_name, KINDS_DOC, MIN_RESOLUTION_DOC,
};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DependenciesParams {
    /// The symbol name to analyze
    pub symbol: String,
    /// Optional: narrow to a specific file path if symbol name is ambiguous
    pub file: Option<String>,
    /// How many hops to traverse. Default 3. Max 10.
    pub depth: Option<i32>,
    #[schemars(description = KINDS_DOC)]
    pub kinds: Option<Vec<String>>,
    #[schemars(description = MIN_RESOLUTION_DOC)]
    pub min_resolution: Option<String>,
}

pub fn execute(graph: &InvarGraph, params: DependenciesParams) -> Result<String, String> {
    let depth = params.depth.unwrap_or(3).clamp(1, 10) as usize;

    let mask = mask_from_names(&params.kinds)?;
    let min_resolution = resolution_from_name(&params.min_resolution)?;
    let edges = query::dependencies(
        graph,
        &params.symbol,
        params.file.as_deref(),
        Some(depth),
        mask,
        min_resolution,
    )
    .map_err(|e| format!("{e}"))?;

    let body = context_builder::format_dependencies(
        graph,
        &params.symbol,
        params.file.as_deref(),
        depth,
        &edges,
    );
    Ok(match absent_kinds_warning(&mask, graph) {
        Some(warning) => format!("{warning}\n\n{body}"),
        None => body,
    })
}
