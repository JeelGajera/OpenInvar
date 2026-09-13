//! OpenInvar MCP server — exposes graph query tools to coding agents.
//!
//! Uses the official `rmcp` SDK with stdio transport. Agents like Cursor
//! and Claude Code spawn `openinvar serve --stdio` and communicate via
//! JSON-RPC over stdin/stdout.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::ServerCapabilities;
use rmcp::model::ServerInfo;
use rmcp::{tool, tool_handler, tool_router, ServerHandler, ServiceExt};

use openinvar_core::graph::InvarGraph;
use openinvar_store::RocksGraphStore;

use crate::tools::{blast_radius, check_rules, dependencies, graph_diff, refresh_graph, symbol_usages};

/// The OpenInvar MCP server. Holds a loaded graph and tool router.
#[derive(Clone)]
pub struct InvarMcpServer {
    graph: Arc<RwLock<InvarGraph>>,
    repo_root: PathBuf,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl InvarMcpServer {
    /// Create a new server by loading the graph from `.openinvar/db`.
    pub fn new(repo_root: PathBuf) -> Result<Self, String> {
        let graph = load_graph(&repo_root)?;
        Ok(Self {
            graph: Arc::new(RwLock::new(graph)),
            repo_root,
            tool_router: Self::tool_router(),
        })
    }

    /// Given a symbol name, returns all symbols that depend on it and would
    /// be affected by changes. Resolves aliases. Tracks property-level access.
    #[tool(
        name = "get_blast_radius",
        description = "Given a symbol name, returns all symbols that depend on it across TypeScript, JavaScript, Python, Rust, Go, C, and C++ codebases. Resolves import aliases (including `as` aliases in all languages). Tracks property-level field access."
    )]
    async fn get_blast_radius(
        &self,
        params: Parameters<blast_radius::BlastRadiusParams>,
    ) -> Result<String, String> {
        let graph = self
            .graph
            .read()
            .map_err(|_| "graph lock poisoned".to_string())?;
        blast_radius::execute(&graph, params.0)
    }

    /// Returns everything a given symbol depends on — its full dependency tree.
    #[tool(
        name = "get_dependencies",
        description = "Returns everything a given symbol depends on — its full dependency tree."
    )]
    async fn get_dependencies(
        &self,
        params: Parameters<dependencies::DependenciesParams>,
    ) -> Result<String, String> {
        let graph = self
            .graph
            .read()
            .map_err(|_| "graph lock poisoned".to_string())?;
        dependencies::execute(&graph, params.0)
    }

    /// Finds all usages of a symbol across the codebase, including under
    /// aliases and re-exports.
    #[tool(
        name = "get_symbol_usages",
        description = "Finds all usages of a symbol across the codebase, including under aliases and re-exports. Use this when you need to find all references before renaming or deleting a symbol."
    )]
    async fn get_symbol_usages(
        &self,
        params: Parameters<symbol_usages::SymbolUsagesParams>,
    ) -> Result<String, String> {
        let graph = self
            .graph
            .read()
            .map_err(|_| "graph lock poisoned".to_string())?;
        symbol_usages::execute(&graph, params.0)
    }

    /// What changed between two recorded snapshots, and what it broke.
    #[tool(
        name = "graph_diff",
        description = "Compares two recorded snapshots and reports what changed and what it broke: removed symbols still referenced elsewhere, removed API surface, renames, and changed signatures. Defaults to comparing HEAD against the working tree. Reads snapshots recorded by `openinvar analyze --snapshot`; it never re-analyzes, so the answer depends only on the revisions named. Reports how much of the result is gate-safe."
    )]
    async fn graph_diff(
        &self,
        params: Parameters<graph_diff::GraphDiffParams>,
    ) -> Result<String, String> {
        graph_diff::execute(&self.repo_root, params.0)
    }

    /// Evaluate the repository's own rules against a change.
    #[tool(
        name = "check_rules",
        description = "Evaluates the constraints in .openinvar/rules.toml — forbidden dependencies, forbidden references, field removals, and fan-in limits — and reports which are violated. Pass base and head to also check rules that need a change. A rule too weakly resolved to judge is reported as undecided and a rule needing a change that was not supplied is reported as skipped; neither is a pass."
    )]
    async fn check_rules(
        &self,
        params: Parameters<check_rules::CheckRulesParams>,
    ) -> Result<String, String> {
        let graph = self
            .graph
            .read()
            .map_err(|_| "graph lock poisoned".to_string())?;
        check_rules::execute(&self.repo_root, &graph, params.0)
    }

    /// Rebuild and persist the graph index. Agents can call this after code changes.
    #[tool(
        name = "refresh_graph_index",
        description = "Re-analyzes the repository and updates the persisted graph index. Supports include/exclude filters and .gitignore respect."
    )]
    async fn refresh_graph_index(
        &self,
        params: Parameters<refresh_graph::RefreshGraphParams>,
    ) -> Result<String, String> {
        let (new_graph, result) = refresh_graph::execute(&self.repo_root, params.0)?;
        {
            let mut graph = self
                .graph
                .write()
                .map_err(|_| "graph lock poisoned".to_string())?;
            *graph = new_graph;
        }

        Ok(format!(
            "Graph index refreshed successfully.\nFiles indexed: {}\nSymbols: {}\nRelationships: {}\nAlias chains: {}\nDiagnostics: {}",
            result.files_indexed,
            result.symbols,
            result.relationships,
            result.alias_chains,
            result.diagnostics
        ))
    }
}

#[tool_handler]
impl ServerHandler for InvarMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "openinvar",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "OpenInvar is a deterministic code intelligence engine. \
                 Use get_blast_radius to find what will break if you change a symbol, \
                 get_dependencies to see what a symbol depends on, \
                 get_symbol_usages to find every usage including aliased imports, \
                 graph_diff to see what a change broke, \
                 check_rules to enforce this repository's own constraints, \
                 and refresh_graph_index after repository changes. \
                 Results carry their resolution: an answer marked structural was matched \
                 within one file and cannot see across files, so an empty result from it \
                 is not evidence that nothing depends on the symbol.",
            )
    }
}

/// Start the MCP server over stdio transport.
pub async fn serve_stdio(repo_root: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let server = InvarMcpServer::new(repo_root)?;

    let transport = rmcp::transport::io::stdio();
    let running_service = server.serve(transport).await?;

    // Wait until the client disconnects
    running_service.waiting().await?;

    Ok(())
}

// ── helpers ──────────────────────────────────────────────────

fn load_graph(repo_root: &Path) -> Result<InvarGraph, String> {
    let db = repo_root.join(".openinvar").join("graph.db");
    if !db.exists() {
        // Not an error — agent can call refresh_graph_index later
        eprintln!(
            "[openinvar] No graph at {}. Starting with empty graph. \
             Run `openinvar analyze` or call refresh_graph_index tool.",
            db.display()
        );
        return Ok(InvarGraph::new());
    }
    let store = RocksGraphStore::open(&db).map_err(|e| format!("failed to open store: {e}"))?;
    store
        .load_graph()
        .map_err(|e| format!("failed to load graph: {e}"))
}
