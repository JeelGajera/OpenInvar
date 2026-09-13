//! MCP tool: get_blast_radius
//!
//! Given a symbol name, returns all symbols that depend on it and would be
//! affected by changes. Resolves aliases. Tracks property-level access.

use schemars::JsonSchema;
use serde::Deserialize;

use openinvar_core::graph::InvarGraph;
use openinvar_core::query;

use crate::context_builder;
use crate::tools::kinds::{
    absent_kinds_warning, mask_from_names, resolution_from_name, KINDS_DOC, MIN_RESOLUTION_DOC,
};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BlastRadiusParams {
    /// The symbol name to analyze (e.g. 'UserPayload', 'authService', 'processOrder')
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

pub fn execute(graph: &InvarGraph, params: BlastRadiusParams) -> Result<String, String> {
    let depth = params.depth.unwrap_or(3).clamp(1, 10) as usize;

    let mask = mask_from_names(&params.kinds)?;
    let min_resolution = resolution_from_name(&params.min_resolution)?;
    let edges = query::blast_radius(
        graph,
        &params.symbol,
        params.file.as_deref(),
        Some(depth),
        mask,
        min_resolution,
    )
    .map_err(|e| format!("{e}"))?;

    let body = context_builder::format_blast_radius(
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
