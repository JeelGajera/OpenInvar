use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use openinvar_core::scan::{
    detect_language_from_extension, is_any_supported_source_file, load_root_gitignore_rules,
    parse_csv_patterns, should_include_relative_path, walk_source_files_with_config, GitignoreRule,
    ScanConfig,
};
use openinvar_lang::analyze_files;
use openinvar_store::RocksGraphStore;
use notify::{RecursiveMode, Watcher};

use crate::output;

const DEBOUNCE: Duration = Duration::from_millis(300);

pub fn run(
    path: &str,
    include_csv: Option<&str>,
    exclude_csv: Option<&str>,
    respect_gitignore: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = super::normalize_path(
        &std::fs::canonicalize(path).map_err(|e| format!("cannot access '{}': {}", path, e))?,
    );

    output::banner("watch");
    output::info(&format!(
        "Watching {}",
        output::file_path(&root.display().to_string())
    ));
    output::dim_line("Press Ctrl+C to stop.");
    output::blank();

    let scan_config = ScanConfig {
        include_patterns: parse_csv_patterns(include_csv),
        exclude_patterns: parse_csv_patterns(exclude_csv),
        respect_gitignore,
    };
    let gitignore_rules = if scan_config.respect_gitignore {
        load_root_gitignore_rules(&root)
    } else {
        Vec::new()
    };

    // ── initial analysis ─────────────────────────────────────
    let start = Instant::now();
    let files = walk_source_files_with_config(&root, &scan_config, is_any_supported_source_file)
        .map_err(|e| format!("initial scan failed: {e}"))?;
    let repo_ir =
        analyze_files(&root, &files).map_err(|e| format!("initial analysis failed: {e}"))?;
    let (graph, stats) = super::analyze::build_graph(&repo_ir);

    let db = super::db_path(&root);
    if let Some(parent) = db.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let store = RocksGraphStore::open(&db).map_err(|e| format!("failed to open store: {e}"))?;
    store
        .save_graph(&graph)
        .map_err(|e| format!("failed to persist graph: {e}"))?;

    output::success(&format!(
        "Initial analysis complete — {} symbols, {} edges ({:.0?})",
        stats.symbols,
        stats.relationships,
        start.elapsed(),
    ));
    output::blank();

    // ── start watcher ────────────────────────────────────────
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(tx)
        .map_err(|e| format!("failed to create file watcher: {e}"))?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|e| format!("failed to start watching: {e}"))?;

    let mut pending: HashSet<String> = HashSet::new();
    let mut last_event = Instant::now();

    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(event)) => {
                for path in &event.paths {
                    if is_watchable(path, &root, &scan_config, &gitignore_rules) {
                        if let Some(rel) = path
                            .strip_prefix(&root)
                            .ok()
                            .map(|p| p.to_string_lossy().replace('\\', "/"))
                        {
                            pending.insert(rel);
                        }
                        last_event = Instant::now();
                    }
                }
            }
            Ok(Err(e)) => {
                output::warning(&format!("Watch error: {e}"));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // check debounce window
                if !pending.is_empty() && last_event.elapsed() >= DEBOUNCE {
                    let files: Vec<String> = pending.drain().collect();
                    handle_change(&root, &store, &files, &scan_config);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                output::info("Watcher disconnected.");
                break;
            }
        }
    }

    Ok(())
}

fn handle_change(root: &Path, store: &RocksGraphStore, files: &[String], scan_config: &ScanConfig) {
    let file_list = files
        .iter()
        .map(|f| output::file_path(f))
        .collect::<Vec<_>>()
        .join(", ");
    let ts = output::dim(&format!("[{}]", output::timestamp()));
    println!("  {ts} Changed: {file_list}");
    print!("  {}      Re-analyzing … ", output::dim(""));

    let start = Instant::now();
    let scan_res = walk_source_files_with_config(root, scan_config, is_any_supported_source_file);
    let repo_ir_res = scan_res.and_then(|files| {
        analyze_files(root, &files).map_err(|e| std::io::Error::other(format!("{e}")))
    });

    match repo_ir_res {
        Ok(repo_ir) => {
            let (graph, stats) = super::analyze::build_graph(&repo_ir);
            match store.save_graph(&graph) {
                Ok(()) => {
                    println!(
                        "{} ({} symbols, {} edges, {:.0?})",
                        output::green("done"),
                        stats.symbols,
                        stats.relationships,
                        start.elapsed(),
                    );
                }
                Err(e) => {
                    println!("{}", output::red(&format!("persist failed: {e}")));
                }
            }
        }
        Err(e) => {
            println!("{}", output::red(&format!("analysis failed: {e}")));
        }
    }
    println!();
}

fn is_watchable(
    path: &Path,
    root: &Path,
    scan_config: &ScanConfig,
    gitignore_rules: &[GitignoreRule],
) -> bool {
    // skip unsupported extensions quickly
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    if detect_language_from_extension(ext).is_none() {
        return false;
    }

    if let Ok(relative) = path.strip_prefix(root) {
        let rel_str = relative.to_string_lossy().replace('\\', "/");
        return should_include_relative_path(&rel_str, false, scan_config, gitignore_rules);
    }

    false
}
