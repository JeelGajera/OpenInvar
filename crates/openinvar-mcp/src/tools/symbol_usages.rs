//! MCP tool: get_symbol_usages
//!
//! Finds all usages of a symbol across the codebase, including under aliases
//! and re-exports. Use this when you need to find all references before
//! renaming or deleting a symbol.

use schemars::JsonSchema;
use serde::Deserialize;

use openinvar_core::graph::InvarGraph;
use openinvar_core::query;

use crate::context_builder;
use crate::tools::kinds::{
    absent_kinds_warning, mask_from_names, resolution_from_name, KINDS_DOC, MIN_RESOLUTION_DOC,
};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SymbolUsagesParams {
    /// The symbol name to find
    pub symbol: String,
    /// Optional: narrow to a specific file path if symbol name is ambiguous
    pub file: Option<String>,
    /// Include usages under aliased imports. Default true. Never set false.
    pub include_aliases: Option<bool>,
    #[schemars(description = KINDS_DOC)]
    pub kinds: Option<Vec<String>>,
    #[schemars(description = MIN_RESOLUTION_DOC)]
    pub min_resolution: Option<String>,
}

pub fn execute(graph: &InvarGraph, params: SymbolUsagesParams) -> Result<String, String> {
    let include_aliases = params.include_aliases.unwrap_or(true);

    let mask = mask_from_names(&params.kinds)?;
    let min_resolution = resolution_from_name(&params.min_resolution)?;
    let edges = query::symbol_usages(
        graph,
        &params.symbol,
        params.file.as_deref(),
        include_aliases,
        mask,
        min_resolution,
    )
    .map_err(|e| format!("{e}"))?;

    let body = context_builder::format_symbol_usages(
        graph,
        &params.symbol,
        params.file.as_deref(),
        &edges,
    );
    Ok(match absent_kinds_warning(&mask, graph) {
        Some(warning) => format!("{warning}\n\n{body}"),
        None => body,
    })
}
