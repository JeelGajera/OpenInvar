use crate::output;

pub fn run(port: u16, stdio: bool) -> Result<(), Box<dyn std::error::Error>> {
    if stdio {
        return run_stdio();
    }

    // TCP transport — not yet supported
    output::banner("serve");
    output::info(&format!("Transport: TCP on port {port}"));
    output::blank();
    output::warning("TCP transport is not yet implemented.");
    output::dim_line("Use --stdio for agent integration (Cursor, Claude Code, Codex).");
    output::blank();
    output::dim_line("  openinvar serve --stdio");
    output::blank();
    Ok(())
}

fn run_stdio() -> Result<(), Box<dyn std::error::Error>> {
    // ── 1. Route ALL tracing/log output to stderr ──────────────
    // rmcp uses tracing internally. Without a subscriber, tracing
    // may buffer or drop events. With a subscriber writing to
    // stdout, it would corrupt the JSON-RPC stream.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .with_ansi(false) // no ANSI colors in log output
        .with_target(false) // compact format
        .init();

    // ── 2. Ensure panics go to stderr, not stdout ─────────────
    std::panic::set_hook(Box::new(|info| {
        eprintln!("[openinvar] PANIC: {info}");
    }));

    // ── 3. Resolve repo root ──────────────────────────────────
    let repo_root = std::env::var("OPENINVAR_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| ".".into()));

    let repo_root = match std::fs::canonicalize(&repo_root) {
        Ok(p) => super::normalize_path(&p),
        Err(e) => {
            eprintln!(
                "[openinvar] ERROR: cannot access '{}': {e}",
                repo_root.display()
            );
            std::process::exit(1);
        }
    };

    eprintln!("[openinvar] MCP server starting for {}", repo_root.display());

    // ── 4. Start tokio runtime + MCP server ───────────────────
    let rt = tokio::runtime::Runtime::new().map_err(|e| {
        eprintln!("[openinvar] ERROR: failed to start async runtime: {e}");
        format!("failed to start async runtime: {e}")
    })?;

    rt.block_on(async {
        if let Err(e) = openinvar_mcp::server::serve_stdio(repo_root).await {
            eprintln!("[openinvar] ERROR: MCP server failed: {e}");
        }
    });

    Ok(())
}
