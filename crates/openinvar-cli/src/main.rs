use clap::{Parser, Subcommand};

mod commands;
mod output;

#[derive(Parser)]
#[command(
    name = "openinvar",
    version,
    about = "Deterministic integrity checks for AI-written code.",
    long_about = "\
OpenInvar catches structural violations, test tampering and eroded \
contracts in a change — at the edit, at commit, or in CI. It reads the \
change as a graph of symbol relationships; no model participates in \
building that graph or in any gating decision, so identical input \
produces identical output.\n\
\n\
QUICK START:\n  \
  openinvar analyze . --snapshot HEAD\n  \
  openinvar audit . --base HEAD --head worktree\n  \
  openinvar check . --diff-only",
    author = "OpenInvar Contributors"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Analyze a codebase and build the symbol relationship graph
    Analyze {
        /// Path to the repository root
        #[arg(default_value = ".")]
        path: String,
        /// Comma-separated include patterns (relative paths/globs)
        #[arg(long)]
        include: Option<String>,
        /// Comma-separated exclude patterns (relative paths/globs)
        #[arg(long)]
        exclude: Option<String>,
        /// Disable .gitignore filtering
        #[arg(long)]
        no_gitignore: bool,

        /// Emit the analysis as JSON on stdout instead of a human summary
        #[arg(long)]
        json: bool,

        /// Also record this analysis under a revision, for `diff` to compare.
        ///
        /// Takes a commit, branch or tag — resolved to its SHA, so the
        /// snapshot still means something after the branch moves — or
        /// `worktree` for the working tree including uncommitted edits.
        #[arg(long, value_name = "REV")]
        snapshot: Option<String>,

        /// How many revision snapshots to keep, oldest dropped first.
        #[arg(long, value_name = "N", default_value_t = 10)]
        keep_snapshots: usize,
    },

    /// Show what changed between two recorded revisions
    Diff {
        /// Path to the repository root
        #[arg(default_value = ".")]
        path: String,

        /// The revision to compare from. Must already be recorded with
        /// `analyze --snapshot`.
        #[arg(long, default_value = "HEAD")]
        base: String,

        /// The revision to compare to. `worktree` is the working tree
        /// including uncommitted edits.
        #[arg(long, default_value = "worktree")]
        head: String,

        /// Emit the delta as JSON on stdout instead of a human summary
        #[arg(long)]
        json: bool,
    },

    /// What depends on one file — the question a pre-edit hook asks
    ///
    /// A hook fires knowing only a path, with no symbol to ask about yet.
    /// A path outside the graph is reported as having no dependents rather
    /// than erroring, so a hook does not break on the first untracked file.
    Impact {
        /// File to inspect, absolute or relative to the repository root
        file: String,

        /// Path to the repository root
        #[arg(long, default_value = ".")]
        path: String,

        /// Traversal depth (default: 3, max: 10)
        #[arg(long, short, default_value = "3")]
        depth: usize,

        /// Only follow these relationship kinds (repeatable).
        #[arg(long = "kind", value_name = "KIND")]
        kind: Vec<String>,

        /// Lowest resolution an edge may have to be followed.
        #[arg(long, value_name = "LEVEL", default_value = "structural")]
        min_confidence: String,

        /// Emit the result as JSON on stdout instead of a human summary
        #[arg(long)]
        json: bool,
    },

    /// Detect reward-hacking in a change
    ///
    /// Deterministic, with no model involved. Reports changes that look like
    /// they were made to pass a check rather than to work: a test that
    /// stopped covering code it used to, a symbol deleted instead of its
    /// caller fixed, code only a test refers to.
    ///
    /// Exit 0 when nothing was found at the requested severity, 1 when
    /// something was, 2 when the audit could not run. Detectors that were not
    /// run are named in the output — an absent check must not read as a
    /// clean one.
    Audit {
        /// Path to the repository root
        #[arg(default_value = ".")]
        path: String,

        /// The revision to compare from. Must already be recorded with
        /// `analyze --snapshot`.
        #[arg(long, default_value = "HEAD")]
        base: String,

        /// The revision to compare to.
        #[arg(long, default_value = "worktree")]
        head: String,

        /// Lowest severity to report and to exit non-zero on.
        #[arg(long, value_name = "LEVEL", default_value = "warn")]
        severity: String,

        /// Emit the result as JSON on stdout
        #[arg(long)]
        json: bool,
    },

    /// Enforce the rules in openinvar.toml
    ///
    /// Exit status is part of the contract: 0 when nothing was violated,
    /// 1 when a rule was broken on resolved evidence, 2 when the check
    /// could not run at all. A rule whose scope is too weakly resolved to
    /// judge is reported as undecided and does not fail the build.
    Check {
        /// Path to the repository root
        #[arg(default_value = ".")]
        path: String,

        /// Rules file to read. Defaults to <path>/openinvar.toml
        #[arg(long, value_name = "FILE")]
        rules: Option<String>,

        /// Evaluate change-sensitive rules against this revision as "before".
        /// Must be given with --head, and both must already be recorded
        /// with `analyze --snapshot`.
        #[arg(long, value_name = "REV")]
        base: Option<String>,

        /// The revision to compare to. `worktree` is the working tree
        /// including uncommitted edits.
        #[arg(long, value_name = "REV")]
        head: Option<String>,

        /// Shorthand for `--base HEAD --head worktree`.
        ///
        /// What a pre-commit hook wants: judge the uncommitted change against
        /// the last commit. Conflicts with --base/--head.
        #[arg(long, conflicts_with_all = ["base", "head"])]
        diff_only: bool,

        /// Emit the result as JSON on stdout instead of a human summary
        #[arg(long)]
        json: bool,

        /// Treat a missing rules file as a failure.
        ///
        /// Off by default so the command is harmless in a repository that
        /// has written no rules; on in CI, where a rules file that has gone
        /// missing would otherwise pass silently.
        #[arg(long)]
        require_rules: bool,
    },

    /// One markdown report for a pull request comment
    ///
    /// Combines the diff and the rule check into a single comment, with the
    /// verdict on the first line. Exit status matches `check`: 0 clean,
    /// 1 a rule was violated, 2 the report could not be produced.
    Report {
        /// Path to the repository root
        #[arg(default_value = ".")]
        path: String,

        /// The revision to compare from. Must already be recorded with
        /// `analyze --snapshot`.
        #[arg(long, default_value = "HEAD")]
        base: String,

        /// The revision to compare to.
        #[arg(long, default_value = "worktree")]
        head: String,

        /// Rules file to read. Defaults to <path>/openinvar.toml
        #[arg(long, value_name = "FILE")]
        rules: Option<String>,

        /// Treat a missing rules file as a failure.
        ///
        /// Off by default so the report is useful in a repository that has
        /// written no rules; on in CI, where a rules file that has gone
        /// missing would otherwise produce a clean report having enforced
        /// nothing.
        #[arg(long)]
        require_rules: bool,
    },

    /// Which tests exercise a symbol, or a change
    ///
    /// The verify loop: run the tests that cover what you changed instead of
    /// the whole suite, or nothing. Exit status carries the confidence — 0
    /// when the selection is complete enough to run in place of the suite,
    /// 3 when something could be missing, 2 when the question could not be
    /// answered. Naming a subset is a claim that the tests left out cannot
    /// fail, so the caveat is machine-readable rather than only printed.
    Tests {
        /// Symbol to find tests for. Omit and pass --diff to use a change.
        symbol: Option<String>,

        /// Path to the repository root
        #[arg(long, default_value = ".")]
        path: String,

        /// Select tests for a recorded change rather than for one symbol.
        #[arg(long)]
        diff: bool,

        /// With --diff, the revision to compare from.
        #[arg(long, value_name = "REV")]
        base: Option<String>,

        /// With --diff, the revision to compare to.
        #[arg(long, value_name = "REV")]
        head: Option<String>,

        /// How far a change propagates before a test stops counting as
        /// covering it. 0 is direct references only.
        #[arg(long, short, default_value = "3")]
        depth: usize,

        /// Lowest resolution an edge may have to be followed.
        #[arg(long, value_name = "LEVEL", default_value = "resolved")]
        min_confidence: String,

        /// Emit the selection as JSON on stdout
        #[arg(long)]
        json: bool,
    },

    /// A minimal working set for orienting on a symbol or a change
    ///
    /// Emits the symbol, what it depends on, what depends on it, and a
    /// signature for each — the shape of the neighbourhood rather than its
    /// contents. Reports what it cost, and what reading the same files whole
    /// would have cost. Token figures are byte-based estimates: OpenInvar
    /// vendors no tokenizer, because a figure that moved with somebody's model
    /// would not be reproducible.
    Context {
        /// Symbol to orient on. Omit and pass --diff to use a change.
        symbol: Option<String>,

        /// Path to the repository root
        #[arg(long, default_value = ".")]
        path: String,

        /// Orient on a recorded change rather than one symbol.
        #[arg(long)]
        diff: bool,

        /// With --diff, the revision to compare from.
        #[arg(long, value_name = "REV")]
        base: Option<String>,

        /// With --diff, the revision to compare to.
        #[arg(long, value_name = "REV")]
        head: Option<String>,

        /// How many hops of neighbourhood to include, in each direction.
        #[arg(long, short, default_value = "1")]
        depth: usize,

        /// Cap the estimated tokens. Outermost hops are dropped first, and
        /// what was dropped is reported rather than silently cut.
        #[arg(long, value_name = "TOKENS")]
        budget: Option<usize>,

        /// Lowest resolution an edge may have to be followed.
        #[arg(long, value_name = "LEVEL", default_value = "resolved")]
        min_confidence: String,

        /// Emit the working set as JSON on stdout
        #[arg(long)]
        json: bool,
    },

    /// Query the symbol relationship graph
    Query {
        #[command(subcommand)]
        subcommand: QueryCommands,
    },

    /// Watch for file changes and update the graph incrementally
    Watch {
        /// Path to the repository root
        #[arg(default_value = ".")]
        path: String,
        /// Comma-separated include patterns (relative paths/globs)
        #[arg(long)]
        include: Option<String>,
        /// Comma-separated exclude patterns (relative paths/globs)
        #[arg(long)]
        exclude: Option<String>,
        /// Disable .gitignore filtering
        #[arg(long)]
        no_gitignore: bool,
    },

    /// Start the MCP server for agent integration
    Serve {
        /// Port number for TCP transport
        #[arg(long, default_value = "7700")]
        port: u16,

        /// Use stdio transport instead of TCP
        #[arg(long)]
        stdio: bool,
    },

    /// Show graph statistics and status
    Status {
        /// Path to the repository root
        #[arg(default_value = ".")]
        path: String,
    },
}

#[derive(Subcommand)]
enum QueryCommands {
    /// Find all symbols that depend on the target — what will break?
    #[command(name = "blast-radius")]
    BlastRadius {
        /// Symbol name to analyze
        symbol: String,

        /// Narrow to a specific file if symbol name is ambiguous
        #[arg(long, short)]
        file: Option<String>,

        /// Traversal depth (default: 3, max: 10)
        #[arg(long, short, default_value = "3")]
        depth: usize,

        /// Path to the repository root
        #[arg(long, default_value = ".")]
        path: String,

        /// Lowest resolution an edge may have to be followed.
        ///
        /// `resolved` restricts the answer to edges bound through imports,
        /// aliases and declared types — what a gate may act on. `structural`
        /// also includes edges matched by name inside one file.
        #[arg(long, value_name = "LEVEL", default_value = "structural")]
        min_confidence: String,
        /// Only follow these relationship kinds (repeatable).
        /// One of: imports, calls, extends, implements, uses-type,
        /// accesses-property, re-exports, instantiates, tests.
        #[arg(long = "kind", value_name = "KIND")]
        kind: Vec<String>,
    },

    /// Find every usage of a symbol, including under aliases
    #[command(name = "usages")]
    Usages {
        /// Symbol name to find
        symbol: String,

        /// Narrow to a specific file if symbol name is ambiguous
        #[arg(long, short)]
        file: Option<String>,

        /// Path to the repository root
        #[arg(long, default_value = ".")]
        path: String,

        /// Lowest resolution an edge may have to be followed.
        ///
        /// `resolved` restricts the answer to edges bound through imports,
        /// aliases and declared types — what a gate may act on. `structural`
        /// also includes edges matched by name inside one file.
        #[arg(long, value_name = "LEVEL", default_value = "structural")]
        min_confidence: String,
        /// Only follow these relationship kinds (repeatable).
        /// One of: imports, calls, extends, implements, uses-type,
        /// accesses-property, re-exports, instantiates, tests.
        #[arg(long = "kind", value_name = "KIND")]
        kind: Vec<String>,
    },

    /// Find all dependencies of a symbol
    #[command(name = "deps")]
    Deps {
        /// Symbol name to analyze
        symbol: String,

        /// Narrow to a specific file if symbol name is ambiguous
        #[arg(long, short)]
        file: Option<String>,

        /// Traversal depth (default: 3, max: 10)
        #[arg(long, short, default_value = "3")]
        depth: usize,

        /// Path to the repository root
        #[arg(long, default_value = ".")]
        path: String,

        /// Lowest resolution an edge may have to be followed.
        ///
        /// `resolved` restricts the answer to edges bound through imports,
        /// aliases and declared types — what a gate may act on. `structural`
        /// also includes edges matched by name inside one file.
        #[arg(long, value_name = "LEVEL", default_value = "structural")]
        min_confidence: String,
        /// Only follow these relationship kinds (repeatable).
        /// One of: imports, calls, extends, implements, uses-type,
        /// accesses-property, re-exports, instantiates, tests.
        #[arg(long = "kind", value_name = "KIND")]
        kind: Vec<String>,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Analyze {
            path,
            include,
            exclude,
            no_gitignore,
            json,
            snapshot,
            keep_snapshots,
        } => commands::analyze::run(
            &path,
            include.as_deref(),
            exclude.as_deref(),
            !no_gitignore,
            json,
            snapshot.as_deref(),
            keep_snapshots,
        ),

        Commands::Diff {
            path,
            base,
            head,
            json,
        } => commands::diff::run(&path, &base, &head, json),

        // `check` owns its exit status — a gate reads it — so it returns a
        // code rather than a unit, and a violation must not be reported
        // through the same path as a tool error.
        Commands::Check {
            path,
            rules,
            base,
            head,
            diff_only,
            json,
            require_rules,
        } => match commands::check::run(
            &path,
            rules.as_deref(),
            if diff_only { Some("HEAD") } else { base.as_deref() },
            if diff_only {
                Some("worktree")
            } else {
                head.as_deref()
            },
            json,
            require_rules,
        ) {
            Ok(code) => std::process::exit(code),
            Err(e) => {
                output::error(&e.to_string());
                std::process::exit(commands::check::EXIT_UNABLE);
            }
        },

        Commands::Impact {
            file,
            path,
            depth,
            kind,
            min_confidence,
            json,
        } => commands::impact::run(&file, &path, depth, &kind, &min_confidence, json),
        Commands::Report {
            path,
            base,
            head,
            rules,
            require_rules,
        } => match commands::report::run(&path, &base, &head, rules.as_deref(), require_rules) {
            Ok(code) => std::process::exit(code),
            Err(e) => {
                output::error(&e.to_string());
                std::process::exit(commands::check::EXIT_UNABLE);
            }
        },

        Commands::Tests {
            symbol,
            path,
            diff,
            base,
            head,
            depth,
            min_confidence,
            json,
        } => match commands::tests::run(
            symbol.as_deref(),
            &path,
            base.as_deref(),
            head.as_deref(),
            diff,
            depth,
            &min_confidence,
            json,
        ) {
            Ok(code) => std::process::exit(code),
            Err(e) => {
                output::error(&e.to_string());
                std::process::exit(commands::tests::EXIT_UNABLE);
            }
        },

        Commands::Audit {
            path,
            base,
            head,
            severity,
            json,
        } => match commands::audit::run(&path, &base, &head, &severity, json) {
            Ok(code) => std::process::exit(code),
            Err(e) => {
                output::error(&e.to_string());
                std::process::exit(commands::audit::EXIT_UNABLE);
            }
        },

        Commands::Context {
            symbol,
            path,
            diff,
            base,
            head,
            depth,
            budget,
            min_confidence,
            json,
        } => match commands::context::run(
            symbol.as_deref(),
            &path,
            diff,
            base.as_deref(),
            head.as_deref(),
            depth,
            budget,
            &min_confidence,
            json,
        ) {
            Ok(code) => std::process::exit(code),
            Err(e) => {
                output::error(&e.to_string());
                std::process::exit(commands::context::EXIT_UNABLE);
            }
        },

        Commands::Query { subcommand } => match subcommand {
            QueryCommands::BlastRadius {
                symbol,
                file,
                depth,
                path,
                kind,
                min_confidence,
            } => commands::query::run_blast_radius(
                &symbol,
                file.as_deref(),
                depth,
                &path,
                &kind,
                &min_confidence,
            ),
            QueryCommands::Usages {
                symbol,
                file,
                path,
                kind,
                min_confidence,
            } => commands::query::run_usages(
                &symbol,
                file.as_deref(),
                &path,
                &kind,
                &min_confidence,
            ),
            QueryCommands::Deps {
                symbol,
                file,
                depth,
                path,
                kind,
                min_confidence,
            } => commands::query::run_deps(
                &symbol,
                file.as_deref(),
                depth,
                &path,
                &kind,
                &min_confidence,
            ),
        },

        Commands::Watch {
            path,
            include,
            exclude,
            no_gitignore,
        } => commands::watch::run(&path, include.as_deref(), exclude.as_deref(), !no_gitignore),

        Commands::Serve { port, stdio } => commands::serve::run(port, stdio),

        Commands::Status { path } => commands::status::run(&path),
    };

    if let Err(e) = result {
        output::error(&e.to_string());
        std::process::exit(1);
    }
}
