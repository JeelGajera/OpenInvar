//! `openinvar diff` — what changed between two recorded revisions.
//!
//! A pure query over stored snapshots. `diff` never analyses: it compares two
//! graphs that `analyze --snapshot` already recorded, so the same pair of
//! revisions always produces the same answer and running it costs a read
//! rather than a full re-analysis. A revision that was never recorded is an
//! error naming the command that would record it, because silently analysing
//! one side would make the result depend on the working tree at the moment the
//! command happened to run.

use std::collections::BTreeSet;

use openinvar_core::delta::{self, Continuation, GraphDelta};
use openinvar_core::findings::{self, DiffFindings, FindingKind};
use openinvar_core::ir::Resolution;
use openinvar_store::RocksGraphStore;

use crate::output;

/// Version of the JSON contract.
///
/// Hooks, CI and the MCP tools read this output, so it is versioned from its
/// first release rather than when the first breaking change is needed.
const SCHEMA_VERSION: u32 = 1;

pub fn run(
    path: &str,
    base: &str,
    head: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = super::normalize_path(
        &std::fs::canonicalize(path).map_err(|e| format!("cannot access '{}': {}", path, e))?,
    );

    let base_rev = openinvar_store::revision::resolve(&root, base)?;
    let head_rev = openinvar_store::revision::resolve(&root, head)?;

    let store = RocksGraphStore::open(&super::db_path(&root))
        .map_err(|e| format!("failed to open store: {e}"))?;

    let before = load(&store, &base_rev, base)?;
    let after = load(&store, &head_rev, head)?;
    let delta = delta::compute(&before, &after);
    let found = findings::derive(&before, &after, &delta);

    if json {
        println!("{}", to_json(&base_rev, &head_rev, &delta, &found));
        return Ok(());
    }

    report(&base_rev, &head_rev, &delta, &found);
    Ok(())
}

/// Load one side, or explain how to record it.
///
/// The error names both the revision as given and as resolved: a user who
/// typed `HEAD` needs to see which commit that was, or the message describes a
/// snapshot they believe they took.
fn load(
    store: &RocksGraphStore,
    resolved: &str,
    given: &str,
) -> Result<openinvar_core::graph::InvarGraph, String> {
    let snapshot = store.load_revision(resolved).map_err(|_| {
        let shown = if resolved == given {
            resolved.to_string()
        } else {
            format!("{given} ({resolved})")
        };
        format!(
            "no snapshot recorded for {shown}. \
             Record one with: openinvar analyze . --snapshot {given}"
        )
    })?;
    snapshot
        .into_graph()
        .map_err(|e| format!("snapshot for '{resolved}' could not be read: {e}"))
}

fn short(revision: &str) -> String {
    if revision.len() == 40 && revision.chars().all(|c| c.is_ascii_hexdigit()) {
        revision[..8].to_string()
    } else {
        revision.to_string()
    }
}

fn how(continuation: Continuation) -> &'static str {
    match continuation {
        Continuation::Renamed => "renamed",
        Continuation::Moved => "moved",
        Continuation::RenamedAndMoved => "renamed and moved",
    }
}

fn report(base: &str, head: &str, delta: &GraphDelta, found: &DiffFindings) {
    output::banner("diff");
    output::info(&format!("{} → {}", short(base), short(head)));
    output::blank();

    if delta.is_empty() {
        output::section("Result");
        output::stat("Changed", "nothing — the two graphs are identical");
        return;
    }

    output::section("Summary");
    output::stat_highlight("Symbols added", &delta.added_symbols.len().to_string());
    output::stat_highlight("Symbols removed", &delta.removed_symbols.len().to_string());
    output::stat_highlight("Symbols renamed or moved", &delta.continuities.len().to_string());
    output::stat_highlight("Signatures changed", &delta.signature_changes.len().to_string());
    output::stat_highlight("Edges added", &delta.added_edges.len().to_string());
    output::stat_highlight("Edges removed", &delta.removed_edges.len().to_string());

    if !delta.removed_symbols.is_empty() {
        output::section("Symbols Removed");
        for symbol in &delta.removed_symbols {
            output::stat(
                &format!("  {}", symbol.name),
                &format!("{}:{}", symbol.file, symbol.line_start),
            );
        }
    }

    if !delta.continuities.is_empty() {
        // Reported separately from add/remove because that is the entire point
        // of pairing them: a rename is one change, not a destruction and an
        // unrelated creation.
        output::section("Symbols Renamed or Moved");
        for continuity in &delta.continuities {
            output::stat(
                &format!("  {} → {}", continuity.before.name, continuity.after.name),
                &format!(
                    "{} ({} → {})",
                    how(continuity.how),
                    continuity.before.file,
                    continuity.after.file
                ),
            );
        }
    }

    if !delta.signature_changes.is_empty() {
        output::section("Signatures Changed");
        for change in &delta.signature_changes {
            output::stat(
                &format!("  {}", change.before.name),
                &format!("{}:{}", change.after.file, change.after.line_start),
            );
        }
    }

    if !delta.added_symbols.is_empty() {
        output::section("Symbols Added");
        for symbol in &delta.added_symbols {
            output::stat(
                &format!("  {}", symbol.name),
                &format!("{}:{}", symbol.file, symbol.line_start),
            );
        }
    }

    // ── findings ─────────────────────────────────────────────
    //
    // Ahead of the raw counts, because a broken reference is what the reader
    // came for and a list of added symbols is not.
    let broken: Vec<_> = found.of_kind(FindingKind::BrokenEdge).collect();
    if !broken.is_empty() {
        output::section("Broken References");
        for finding in &broken {
            output::stat(
                &format!("  {}", finding.name),
                &format!(
                    "removed, still referenced from {} place(s){}",
                    finding.referrers.len(),
                    if finding.resolution == Resolution::Resolved {
                        ""
                    } else {
                        " — structural, advisory only"
                    }
                ),
            );
            for referrer in &finding.referrers {
                output::dim_line(&format!("      {}:{}", referrer.file, referrer.line));
            }
        }
    }

    let orphans: Vec<_> = found.of_kind(FindingKind::Orphaned).collect();
    if !orphans.is_empty() {
        output::section("Orphaned");
        for finding in &orphans {
            output::stat(&format!("  {}", finding.name), "nothing refers to it any more");
        }
    }

    let api: Vec<_> = found.of_kind(FindingKind::ApiSurfaceRemoved).collect();
    if !api.is_empty() {
        output::section("API Surface Removed");
        for finding in &api {
            output::stat(
                &format!("  {}", finding.name),
                &format!("{}:{}", finding.file, finding.line),
            );
        }
    }

    // ── what a gate may act on ───────────────────────────────
    //
    // Reported on every diff, not only when it is bad news. A reader deciding
    // whether to trust this answer needs to know how much of it is resolved,
    // and a figure that appears only sometimes is one nobody learns to read.
    output::section("Confidence");
    let resolved_edges = delta
        .added_edges
        .iter()
        .chain(delta.removed_edges.iter())
        .filter(|e| e.resolution == Resolution::Resolved)
        .count();
    let total_edges = delta.added_edges.len() + delta.removed_edges.len();

    match total_edges {
        0 => output::stat("Edge changes", "none"),
        total => output::stat(
            "Resolved edge changes",
            &format!("{resolved_edges} of {total}"),
        ),
    }
    if !delta::has_resolved_changes(delta) {
        output::dim_line("  Nothing here is gate-safe: every change is structural, so a gate");
        output::dim_line("  must fail open rather than draw a conclusion from this diff.");
    }

    match found.changed_file_coverage.percent() {
        Some(percent) => output::stat(
            "Changed-file coverage",
            &format!("{percent:.1}% of {} edge(s)", found.changed_file_coverage.total()),
        ),
        None => output::stat("Changed-file coverage", "no edges in the changed files"),
    }
    output::stat(
        "Findings a gate may act on",
        &format!("{} of {}", found.gate_safe().count(), found.findings.len()),
    );

    let kinds: BTreeSet<String> = delta::edge_kinds(delta)
        .into_iter()
        .map(|k| format!("{k:?}").to_lowercase())
        .collect();
    if !kinds.is_empty() {
        output::stat(
            "Edge kinds touched",
            &kinds.into_iter().collect::<Vec<_>>().join(", "),
        );
    }
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

fn symbol_json(symbol: &openinvar_core::ir::Symbol) -> String {
    format!(
        r#"{{"id":"{}","name":"{}","kind":"{:?}","file":"{}","line":{}}}"#,
        escape(&symbol.id),
        escape(&symbol.name),
        symbol.kind,
        escape(&symbol.file),
        symbol.line_start
    )
}

fn edge_json(edge: &delta::EdgeRef) -> String {
    format!(
        r#"{{"from":"{}","to":"{}","kind":"{:?}","file":"{}","line":{},"resolution":"{}"}}"#,
        escape(&edge.from),
        escape(&edge.to),
        edge.kind,
        escape(&edge.file),
        edge.line,
        edge.resolution.as_str()
    )
}

fn array(items: Vec<String>) -> String {
    format!("[{}]", items.join(","))
}

fn finding_json(finding: &openinvar_core::findings::Finding) -> String {
    let referrers = array(
        finding
            .referrers
            .iter()
            .map(|r| {
                format!(
                    r#"{{"symbol":"{}","file":"{}","line":{},"kind":"{:?}","resolution":"{}"}}"#,
                    escape(&r.symbol),
                    escape(&r.file),
                    r.line,
                    r.kind,
                    r.resolution.as_str()
                )
            })
            .collect(),
    );
    format!(
        r#"{{"kind":"{}","symbol":"{}","name":"{}","file":"{}","line":{},"resolution":"{}","referrers":{}}}"#,
        finding.kind.as_str(),
        escape(&finding.symbol),
        escape(&finding.name),
        escape(&finding.file),
        finding.line,
        finding.resolution.as_str(),
        referrers
    )
}

fn to_json(base: &str, head: &str, delta: &GraphDelta, found: &DiffFindings) -> String {
    let continuities = array(
        delta
            .continuities
            .iter()
            .map(|c| {
                format!(
                    r#"{{"how":"{}","before":{},"after":{}}}"#,
                    how(c.how),
                    symbol_json(&c.before),
                    symbol_json(&c.after)
                )
            })
            .collect(),
    );
    let signature_changes = array(
        delta
            .signature_changes
            .iter()
            .map(|c| {
                format!(
                    r#"{{"before":{},"after":{}}}"#,
                    symbol_json(&c.before),
                    symbol_json(&c.after)
                )
            })
            .collect(),
    );

    format!(
        concat!(
            r#"{{"schema":{},"base":"{}","head":"{}","#,
            r#""symbols":{{"added":{},"removed":{},"continuities":{},"signature_changes":{}}},"#,
            r#""edges":{{"added":{},"removed":{}}},"#,
            r#""findings":{},"#,
            r#""summary":{{"symbols_added":{},"symbols_removed":{},"symbols_continued":{},"#,
            r#""signatures_changed":{},"edges_added":{},"edges_removed":{},"has_resolved_changes":{},"#,
            r#""findings_total":{},"findings_gate_safe":{},"changed_file_coverage_percent":{}}}}}"#
        ),
        SCHEMA_VERSION,
        escape(base),
        escape(head),
        array(delta.added_symbols.iter().map(symbol_json).collect()),
        array(delta.removed_symbols.iter().map(symbol_json).collect()),
        continuities,
        signature_changes,
        array(delta.added_edges.iter().map(edge_json).collect()),
        array(delta.removed_edges.iter().map(edge_json).collect()),
        array(found.findings.iter().map(finding_json).collect()),
        delta.added_symbols.len(),
        delta.removed_symbols.len(),
        delta.continuities.len(),
        delta.signature_changes.len(),
        delta.added_edges.len(),
        delta.removed_edges.len(),
        delta::has_resolved_changes(delta),
        found.findings.len(),
        found.gate_safe().count(),
        match found.changed_file_coverage.percent() {
            Some(percent) => format!("{percent:.1}"),
            None => "null".to_string(),
        },
    )
}
