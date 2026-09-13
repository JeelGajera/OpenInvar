# openinvar-mcp

MCP server crate for exposing OpenInvar queries to coding agents.

`openinvar-mcp` loads a persisted graph and registers MCP tools so agents can ask blast-radius, dependencies, and symbol-usage questions over standard MCP transports.

## Exposed tools

- `get_blast_radius`
- `get_dependencies`
- `get_symbol_usages`
- `refresh_graph_index`

`refresh_graph_index` lets agents trigger a re-analysis and update the in-memory graph without restarting the MCP server.

## Main entrypoint

```rust
use std::path::PathBuf;
use openinvar_mcp::server;

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
server::serve_stdio(PathBuf::from(".")) .await?;
# Ok(())
# }
```

## Notes

- The server expects a graph store at `.openinvar/graph.db` under the repo root.
- Tool formatting for agent-readable output is handled in `context_builder`.
