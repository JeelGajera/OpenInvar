use std::path::Path;
use std::time::Instant;

use openinvar_core::graph::InvarGraph;
use openinvar_core::ir::RepoIR;
use openinvar_core::resolver::AliasResolver;
use openinvar_core::scan::{
    is_any_supported_source_file, parse_csv_patterns, walk_source_files_reporting, ScanConfig,
};
use openinvar_lang::analyze_files;
use openinvar_store::rocksdb::GraphSnapshot;
use openinvar_store::RocksGraphStore;

use crate::commands::json::AnalysisReport;
use crate::output;

/// Human progress reporting, silenced when the caller wants machine output.
///
/// `--json` has to put a parseable document on stdout and nothing else, so
/// every progress line needs suppressing. Routing them through one gate keeps
/// that a single decision rather than a condition repeated at each call site,
/// where one missed branch would corrupt the document.
struct Progress {
    enabled: bool,
}

impl Progress {
    fn banner(&self, subtitle: &str) {
        if self.enabled {
            output::banner(subtitle);
        }
    }
    fn section(&self, title: &str) {
        if self.enabled {
            output::section(title);
        }
    }
    fn info(&self, msg: &str) {
        if self.enabled {
            output::info(msg);
        }
    }
    fn warning(&self, msg: &str) {
        if self.enabled {
            output::warning(msg);
        }
    }
    fn stat(&self, label: &str, value: &str) {
        if self.enabled {
            output::stat(label, value);
        }
    }
    fn stat_highlight(&self, label: &str, value: &str) {
        if self.enabled {
            output::stat_highlight(label, value);
        }
    }
    fn dim_line(&self, msg: &str) {
        if self.enabled {
            output::dim_line(msg);
        }
    }
    fn blank(&self) {
        if self.enabled {
            output::blank();
        }
    }
    fn step(&self, label: &str, detail: &str) {
        if self.enabled {
            output::step(label, detail);
        }
    }
    fn done(&self, msg: &str) {
        if self.enabled {
            output::done(msg);
        }
    }
}

/// What `analyze` was asked to do.
///
/// A struct rather than a parameter list because `--at` made it the eighth
/// argument, and eight positional booleans and `Option<&str>`s at a call site
/// is how the wrong flag gets passed silently.
pub struct Options<'a> {
    pub path: &'a str,
    pub include_csv: Option<&'a str>,
    pub exclude_csv: Option<&'a str>,
    pub respect_gitignore: bool,
    pub json: bool,
    pub snapshot: Option<&'a str>,
    pub keep_snapshots: usize,
    /// Analyse the tree at this revision instead of the working tree.
    pub at: Option<&'a str>,
}

pub fn run(opts: Options<'_>) -> Result<(), Box<dyn std::error::Error>> {
    let Options {
        path,
        include_csv,
        exclude_csv,
        respect_gitignore,
        json,
        snapshot,
        keep_snapshots,
        at,
    } = opts;

    let root = super::normalize_path(
        &std::fs::canonicalize(path).map_err(|e| format!("cannot access '{}': {}", path, e))?,
    );
    let progress = Progress { enabled: !json };

    // `--at` analyses a revision's tree, so the files come from a throwaway
    // checkout rather than from `root`. `root` stays the repository: it is
    // where the store is written, and it is the root the report names.
    //
    // Resolved before the checkout so an unknown revision is reported as an
    // unknown revision rather than as a worktree failure. `resolve_commit`
    // rather than `resolve`: `--at` cannot accept `worktree`, and it carries
    // the reason.
    let at_revision = at
        .map(|revision| openinvar_store::revision::resolve_commit(&root, revision))
        .transpose()?;

    let checkout = match &at_revision {
        Some(sha) => Some(super::worktree::TempWorktree::add(&root, sha)?),
        None => None,
    };
    // Everything that reads files uses this; everything that writes state uses
    // `root`. Keeping the two apart is the whole of `--at`.
    let scan_root = match &checkout {
        Some(worktree) => super::normalize_path(worktree.path()),
        None => root.clone(),
    };

    progress.banner("analyze");
    match &at_revision {
        Some(sha) => progress.info(&format!(
            "Analyzing {} at {}",
            output::file_path(&root.display().to_string()),
            sha
        )),
        None => progress.info(&format!(
            "Analyzing {}",
            output::file_path(&root.display().to_string())
        )),
    }
    progress.blank();

    let start = Instant::now();

    // ── 1. Scan and parse ────────────────────────────────────
    progress.step("Scanning files", "...");
    let scan_config = ScanConfig {
        include_patterns: parse_csv_patterns(include_csv),
        exclude_patterns: parse_csv_patterns(exclude_csv),
        respect_gitignore,
    };

    let scan = walk_source_files_reporting(&scan_root, &scan_config, is_any_supported_source_file)
        .map_err(|e| format!("scan failed: {e}"))?;
    let files = scan.files;
    if files.is_empty() {
        if !scan_config.include_patterns.is_empty() {
            progress.warning("No files matched your --include patterns.");
            progress.dim_line("  Tip: use ** for recursive matching, e.g. 'projects/api/**/*.ts'");
            progress.dim_line(&format!(
                "  Patterns used: {}",
                scan_config.include_patterns.join(", ")
            ));
        } else {
            progress.warning("No source files were found for analysis.");
            progress.dim_line("  Check your path and include/exclude filters, then retry.");
        }
        // A consumer parsing stdout needs a document here too: "nothing
        // matched" is a result, not an absence of one.
        if json {
            let empty = RepoIR {
                root: root.display().to_string(),
                files: Vec::new(),
                language_stats: Default::default(),
            };
            let stats = AnalyzeStats {
                symbols: 0,
                relationships: 0,
                alias_chains: 0,
                aliases: 0,
            };
            println!("{}", AnalysisReport::new(&empty, &stats).to_json()?);
        }
        return Ok(());
    }

    // A directory pruned by a built-in rule is the most common reason a symbol
    // "goes missing", so say so up front rather than leaving the user to guess.
    if !scan.skipped_dirs.is_empty() {
        let names: Vec<&str> = scan.skipped_dirs.iter().map(String::as_str).collect();
        progress.dim_line(&format!(
            "  Skipped by default: {} — pass --include to index them",
            names.join(", ")
        ));
    }

    let mut repo_ir =
        analyze_files(&scan_root, &files).map_err(|e| format!("analysis failed: {e}"))?;

    // `root` is the one absolute path in the report, and under `--at` it would
    // otherwise be the temporary checkout — a path that differs on every run.
    // Every path inside `files` is already relative to it, so restating the
    // repository here is the whole of what determinism needs: two runs over one
    // revision then produce byte-identical output. `at_json_is_identical_across_runs`
    // asserts it rather than trusting this comment.
    if at_revision.is_some() {
        repo_ir.root = root.display().to_string();
    }
    let repo_ir = repo_ir;

    let file_count = repo_ir.files.len();
    let error_count: usize = repo_ir.files.iter().map(|f| f.diagnostics.len()).sum();
    progress.step(
        "Parsed files",
        &format!("{file_count} OK, {error_count} diagnostic(s)"),
    );

    // ── 2. Build graph ───────────────────────────────────────
    let (graph, stats) = build_graph(&repo_ir);
    progress.step(
        "Built graph",
        &format!("{} symbols, {} edges", stats.symbols, stats.relationships),
    );
    progress.step(
        "Resolved aliases",
        &format!(
            "{} alias(es) across {} symbol(s)",
            stats.aliases, stats.alias_chains
        ),
    );

    // ── 3. Persist ───────────────────────────────────────────
    let db = super::db_path(&root);
    if let Some(parent) = db.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let store = RocksGraphStore::open(&db).map_err(|e| format!("failed to open store: {e}"))?;

    // The working graph is what `query`, `check` and `audit` read when nobody
    // names a revision, so it has to keep meaning "the tree as it is now".
    // `--at` analysed something else, and overwriting it would leave every
    // later command answering questions about a past revision without saying
    // so — a wrong answer delivered confidently, which is the one failure this
    // project is built to avoid. So `--at` records its revision and stops.
    if at_revision.is_none() {
        store
            .save_graph(&graph)
            .map_err(|e| format!("failed to persist graph: {e}"))?;

        progress.step(
            "Persisted to",
            &root.join(".openinvar/").display().to_string(),
        );
    }

    // The rebuild is the fix for an upgrade, so say once that the old store is
    // now dead weight. Deleting it is the user's call, not this command's —
    // and after this the read path never mentions it again, so if nothing said
    // it here the directory would sit there indefinitely.
    if let Some(old_store) = super::legacy_store_dir(&root) {
        output::warning(&format!(
            "A store from an earlier release is still at {}. \
             Nothing reads it now; it can be deleted.",
            old_store.display()
        ));
    }

    // Without `--at` the working graph is always written and a revision
    // snapshot is additional, so `analyze --snapshot` leaves the repository
    // queryable exactly as a plain `analyze` does. With `--at`, the revision
    // *is* the output: the name comes from `--at` itself, which is why the two
    // flags conflict rather than combining into a snapshot under one name
    // holding the tree of another.
    let record_under = match &at_revision {
        Some(sha) => Some(Ok(sha.clone())),
        None => snapshot.map(|revision| openinvar_store::revision::resolve(&root, revision)),
    };

    if let Some(resolved) = record_under {
        let resolved = resolved?;
        let snapshot = GraphSnapshot::from_graph(&graph)
            .map_err(|e| format!("failed to build snapshot: {e}"))?;
        store
            .save_revision(&resolved, &snapshot)
            .map_err(|e| format!("failed to record revision '{resolved}': {e}"))?;

        progress.step("Recorded revision", &resolved);

        let dropped = store
            .prune_revisions(keep_snapshots)
            .map_err(|e| format!("failed to apply snapshot retention: {e}"))?;
        if !dropped.is_empty() {
            progress.step(
                "Dropped older revisions",
                &format!("{} (keeping {keep_snapshots})", dropped.len()),
            );
        }
    }

    // ── 4. Summary ───────────────────────────────────────────
    let elapsed = start.elapsed();
    progress.section("Summary");
    progress.stat_highlight("Symbols", &stats.symbols.to_string());
    progress.stat_highlight("Relationships", &stats.relationships.to_string());
    progress.stat_highlight("Files indexed", &file_count.to_string());
    progress.stat_highlight(
        "Aliases",
        &format!(
            "{} (across {} symbol(s))",
            stats.aliases, stats.alias_chains
        ),
    );
    progress.stat(
        "Respect .gitignore",
        if scan_config.respect_gitignore {
            "yes"
        } else {
            "no"
        },
    );
    if !scan_config.include_patterns.is_empty() {
        progress.stat("Include", &scan_config.include_patterns.join(", "));
    }
    if !scan_config.exclude_patterns.is_empty() {
        progress.stat("Exclude", &scan_config.exclude_patterns.join(", "));
    }

    if !repo_ir.language_stats.is_empty() {
        progress.blank();
        let mut langs: Vec<_> = repo_ir.language_stats.iter().collect();
        langs.sort_by(|a, b| b.1.cmp(a.1));
        for (lang, count) in langs {
            let icon = match lang.as_str() {
                "TypeScript" => "🔷",
                "JavaScript" => "🟡",
                "Python" => "🐍",
                "Rust" => "🦀",
                "Go" => "🐹",
                "C" => "⚙",
                "Cpp" => "⚙",
                _ => "•",
            };
            progress.stat(&format!("  {icon} {lang}"), &format!("{count} file(s)"));
        }
    }

    if error_count > 0 {
        progress.blank();

        // Show parse errors
        let errors: Vec<_> = repo_ir
            .files
            .iter()
            .flat_map(|f| {
                f.diagnostics
                    .iter()
                    .filter(|d| d.level == openinvar_core::ir::DiagnosticLevel::Error)
                    .map(move |d| (f.file.as_str(), d))
            })
            .collect();
        if !errors.is_empty() {
            progress.warning(&format!("{} parse error(s)", errors.len()));
            for (file, diag) in &errors {
                let loc = match diag.line {
                    Some(l) => format!("{file}:{l}"),
                    None => file.to_string(),
                };
                progress.dim_line(&format!("  {} — {}", loc, diag.message));
            }
        }

        // Show resolution warnings
        let warnings: Vec<_> = repo_ir
            .files
            .iter()
            .flat_map(|f| {
                f.diagnostics
                    .iter()
                    .filter(|d| d.level == openinvar_core::ir::DiagnosticLevel::Warning)
                    .map(move |d| (f.file.as_str(), d))
            })
            .collect();
        if !warnings.is_empty() {
            progress.warning(&format!("{} resolution warning(s)", warnings.len()));
            for (file, diag) in &warnings {
                let loc = match diag.line {
                    Some(l) => format!("{file}:{l}"),
                    None => file.to_string(),
                };
                progress.dim_line(&format!("  {} — {}", loc, diag.message));
            }
        }

        // Show info count (skipped files etc.) — no detail unless verbose
        let info_count: usize = repo_ir
            .files
            .iter()
            .flat_map(|f| f.diagnostics.iter())
            .filter(|d| d.level == openinvar_core::ir::DiagnosticLevel::Info)
            .count();
        if info_count > 0 {
            progress.dim_line(&format!(
                "  {} info diagnostic(s) (skipped files, policy exclusions)",
                info_count
            ));
        }
    }

    progress.done(&format!("Analysis complete ({:.0?})", elapsed));

    if json {
        println!("{}", AnalysisReport::new(&repo_ir, &stats).to_json()?);
    }

    Ok(())
}

// ── graph construction ───────────────────────────────────────

pub struct AnalyzeStats {
    pub symbols: usize,
    pub relationships: usize,
    /// Symbols that have at least one alias.
    pub alias_chains: usize,
    /// Aliases in total — the number a reader takes "renames" to mean.
    pub aliases: usize,
}

pub fn build_graph(repo_ir: &RepoIR) -> (InvarGraph, AnalyzeStats) {
    let mut graph = InvarGraph::new();
    let resolver = AliasResolver::default();

    // Add all symbols
    for file_ir in &repo_ir.files {
        for symbol in &file_ir.symbols {
            graph.add_symbol(symbol.clone());
        }
    }

    // Add all relationships and populate alias chains
    for file_ir in &repo_ir.files {
        for relationship in &file_ir.relationships {
            graph.add_relationship(relationship);
        }
        graph
            .file_reexports
            .insert(file_ir.file.clone(), file_ir.re_exports.clone());

        // Assertion counts, plus whether this file's language was counted at
        // all. Both are needed: the counts alone cannot distinguish a test with
        // no assertions left from a file this build cannot read assertions in.
        for (symbol, count) in &file_ir.assertions {
            graph.assertions.insert(symbol.clone(), *count);
        }
        graph
            .assertions_counted
            .insert(file_ir.file.clone(), file_ir.assertions_counted);
        if file_ir.references_counted {
            graph
                .unbound_references
                .insert(file_ir.file.clone(), file_ir.unbound_references);
            graph
                .unbound_outside_repository
                .insert(file_ir.file.clone(), file_ir.unbound_outside_repository);
        }

        resolver.ingest_relationships(&graph, &file_ir.relationships);
    }

    let stats = AnalyzeStats {
        symbols: graph.symbols.len(),
        relationships: graph.graph.edge_count(),
        alias_chains: graph.aliased_symbol_count(),
        aliases: graph.alias_count(),
    };

    (graph, stats)
}

pub fn load_graph(repo_root: &Path) -> Result<InvarGraph, Box<dyn std::error::Error>> {
    let db = super::db_path(repo_root);
    if !db.exists() {
        // A store directory from a release that used RocksDB is the one case
        // where "no graph" has a cause worth naming: the graph is there, this
        // build just cannot read it. Saying only "run analyze" would leave the
        // user looking at a `.openinvar/db` directory that plainly exists.
        if let Some(old_store) = super::legacy_store_dir(repo_root) {
            return Err(format!(
                "The store format changed in this release. Run {} to rebuild.\n\
                 The old store at {} is no longer read, and can be deleted.",
                output::bold_cyan("openinvar analyze <path>"),
                old_store.display(),
            )
            .into());
        }
        return Err(format!(
            "No graph found at {}. Run {} first.",
            db.display(),
            output::bold_cyan("openinvar analyze <path>"),
        )
        .into());
    }
    let store = RocksGraphStore::open(&db).map_err(|e| format!("failed to open store: {e}"))?;
    let graph = store
        .load_graph()
        .map_err(|e| format!("failed to load graph: {e}"))?;
    Ok(graph)
}
