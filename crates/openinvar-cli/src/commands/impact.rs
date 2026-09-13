//! `openinvar impact` — what depends on one file.
//!
//! Built for a hook rather than a person. A hook fires before an agent writes
//! to a file and knows only the path: there is no symbol to ask about yet, so
//! `query blast-radius` cannot answer, and guessing a symbol from the filename
//! would answer a different question confidently.
//!
//! Two properties matter more here than anywhere else in the CLI.
//!
//! **It must not fail on an unremarkable input.** A path outside the graph — a
//! new file, a language no adapter handles, a file the analysis excluded — is
//! reported as "no dependents recorded" rather than an error. A hook that
//! errors the first time an agent touches a README is a hook that gets removed
//! from the settings file that afternoon.
//!
//! **It must not overstate.** An empty answer is qualified by whether the
//! graph could have seen a dependent at all: structural regions cannot record
//! a cross-file reference, so "nothing depends on this" from such a graph is a
//! statement about what was resolved, not about the code.

use std::path::Path;

use openinvar_core::graph::InvarGraph;
use openinvar_core::ir::Resolution;
use openinvar_core::query::{self, FileImpact};

use crate::output;

/// Version of the JSON contract. Hooks parse this, so it is versioned from the
/// first release.
const SCHEMA_VERSION: u32 = 1;

/// How many dependents a hook prints before summarising.
///
/// A hook's output is injected into an agent's context window, where a hundred
/// lines of file paths costs more than it informs.
const HOOK_LIMIT: usize = 10;

pub fn run(
    target: &str,
    path: &str,
    depth: usize,
    kinds: &[String],
    min_confidence: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mask = super::query::mask_from_args(kinds)?;
    let floor = match min_confidence {
        "resolved" => Resolution::Resolved,
        "structural" => Resolution::Structural,
        other => {
            return Err(format!(
                "unknown confidence '{other}'. Expected 'resolved' or 'structural'"
            )
            .into())
        }
    };

    let root = super::normalize_path(
        &std::fs::canonicalize(path).map_err(|e| format!("cannot access '{}': {}", path, e))?,
    );
    let graph = super::analyze::load_graph(&root)?;
    let relative = relative_to(&root, target);

    let impact = query::file_impact(&graph, &relative, Some(depth), mask, floor)
        .map_err(|e| e.to_string())?;

    if json {
        println!("{}", to_json(&impact));
        return Ok(());
    }

    report(&graph, &impact);
    Ok(())
}

/// Express `target` the way the graph records files: relative to the root,
/// with forward slashes.
///
/// A hook is handed an absolute path by the agent that invoked it, and the
/// graph stores repository-relative paths. Getting this wrong does not error —
/// it silently finds nothing, which is the failure mode this whole command is
/// written to avoid.
fn relative_to(root: &Path, target: &str) -> String {
    let candidate = Path::new(target);
    let absolute = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let cleaned = std::fs::canonicalize(&absolute).unwrap_or(absolute);

    cleaned
        .strip_prefix(root)
        .unwrap_or(&cleaned)
        .to_string_lossy()
        .replace('\\', "/")
}

fn report(graph: &InvarGraph, impact: &FileImpact) {
    output::banner("impact");
    output::info(&output::file_path(&impact.file));
    output::blank();

    if impact.defined.is_empty() {
        output::section("Result");
        output::stat("Symbols", "none recorded for this file");
        output::dim_line("  The file is new, excluded, or in a language with no adapter.");
        output::dim_line("  Run: openinvar analyze . — if you expect it to be tracked.");
        return;
    }

    output::section("Summary");
    output::stat_highlight("Symbols defined", &impact.defined.len().to_string());
    output::stat_highlight("Dependent edges", &impact.edges.len().to_string());
    output::stat_highlight("Dependent files", &impact.dependents.len().to_string());

    if impact.is_empty() {
        output::blank();
        // The same care `query blast-radius` takes: an empty answer is only
        // "safe" when the graph was capable of producing a non-empty one.
        if impact.blind_spots == 0 {
            output::success("Nothing outside this file depends on it.");
        } else {
            output::warning("No dependents found — but not safe to conclude.");
            output::dim_line(&format!(
                "  {} file(s) were analyzed within-file only, so a reference",
                impact.blind_spots
            ));
            output::dim_line("  from one of them would not appear here.");
        }
        return;
    }

    output::section("Dependent Files");
    for (file, count) in &impact.dependents {
        output::stat(&format!("  {file}"), &format!("{count} edge(s)"));
    }

    let (direct, aliased) = openinvar_core::query::partition_by_alias(graph, &impact.edges);
    if !aliased.is_empty() {
        output::section("Aliased References — HIGH RISK");
        output::dim_line("  These reach this file under a different name, so a text");
        output::dim_line("  search for the original will not find them.");
        for edge in aliased.iter().take(HOOK_LIMIT) {
            output::stat(
                &format!("  {}:{}", edge.file, edge.line),
                edge.alias.as_deref().unwrap_or("aliased"),
            );
        }
    }

    output::blank();
    if !impact.gate_safe {
        output::warning("Some of these edges are structural — advisory, not gate-safe.");
    }
    let _ = direct;
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

fn to_json(impact: &FileImpact) -> String {
    let dependents: Vec<String> = impact
        .dependents
        .iter()
        .map(|(file, count)| format!(r#"{{"file":"{}","edges":{}}}"#, escape(file), count))
        .collect();

    let edges: Vec<String> = impact
        .edges
        .iter()
        .map(|e| {
            format!(
                r#"{{"from":"{}","to":"{}","kind":"{}","file":"{}","line":{},"resolution":"{}","alias":{}}}"#,
                escape(&e.from),
                escape(&e.to),
                openinvar_core::query::kind_name(&e.kind),
                escape(&e.file),
                e.line,
                e.resolution.as_str(),
                match &e.alias {
                    Some(alias) => format!(r#""{}""#, escape(alias)),
                    None => "null".to_string(),
                }
            )
        })
        .collect();

    format!(
        // The counts are emitted as their own fields rather than left to be
        // derived from the arrays. A shell hook without `jq` counts key
        // occurrences, and `"file"` appears once per edge as well as once per
        // dependent — which reported 719 dependent files for a file that has
        // 136 until these existed.
        r#"{{"schema_version":{},"file":"{}","symbols_defined":{},"dependent_file_count":{},"edge_count":{},"gate_safe":{},"blind_spots":{},"dependent_files":[{}],"edges":[{}]}}"#,
        SCHEMA_VERSION,
        escape(&impact.file),
        impact.defined.len(),
        impact.dependents.len(),
        impact.edges.len(),
        impact.gate_safe,
        impact.blind_spots,
        dependents.join(","),
        edges.join(",")
    )
}
