//! MCP tool: graph_diff
//!
//! What changed between two recorded snapshots, and what it broke.
//!
//! A pure read over stored graphs. It never analyses: an agent asking "what
//! did my change break" must get an answer that depends on the two revisions
//! it named, not on whatever happened to be on disk when the tool ran.
//!
//! The output is prose rather than JSON because it is going into a context
//! window, where a model reads a sentence more reliably than it reads a nested
//! object — and where every token spent on punctuation is one not spent on the
//! finding.

use schemars::JsonSchema;
use serde::Deserialize;

use openinvar_core::delta::{self, GraphDelta};
use openinvar_core::findings::{self, DiffFindings, FindingKind};
use openinvar_core::graph::InvarGraph;
use openinvar_core::ir::Resolution;
use openinvar_store::RocksGraphStore;

use std::path::Path;

/// How many items of one kind are listed before the rest are summarised.
const LIST_LIMIT: usize = 15;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GraphDiffParams {
    /// The revision to compare from. A commit, branch or tag, or 'worktree'
    /// for uncommitted work. Defaults to 'HEAD'. Must already be recorded
    /// with `openinvar analyze --snapshot`.
    pub base: Option<String>,
    /// The revision to compare to. Defaults to 'worktree', the working tree
    /// including uncommitted edits.
    pub head: Option<String>,
}

pub fn execute(repo_root: &Path, params: GraphDiffParams) -> Result<String, String> {
    let base = params.base.as_deref().unwrap_or("HEAD");
    let head = params.head.as_deref().unwrap_or("worktree");

    let base_rev = openinvar_store::revision::resolve(repo_root, base)?;
    let head_rev = openinvar_store::revision::resolve(repo_root, head)?;

    let store = RocksGraphStore::open(&repo_root.join(".openinvar").join("graph.db"))
        .map_err(|e| format!("failed to open the graph store: {e}"))?;

    let before = load(&store, &base_rev, base)?;
    let after = load(&store, &head_rev, head)?;
    let delta = delta::compute(&before, &after);
    let found = findings::derive(&before, &after, &delta);

    Ok(render(base, head, &delta, &found))
}

/// Load one side, or say how to record it.
///
/// Recording it here instead would be the tempting shortcut and the wrong one:
/// the answer would then describe the tree at the moment of the call rather
/// than the revision the agent asked about.
fn load(store: &RocksGraphStore, resolved: &str, given: &str) -> Result<InvarGraph, String> {
    let snapshot = store.load_revision(resolved).map_err(|_| {
        let shown = if resolved == given {
            resolved.to_string()
        } else {
            format!("{given} ({resolved})")
        };
        format!(
            "No snapshot recorded for {shown}. Record one first with: \
             openinvar analyze . --snapshot {given} — or call refresh_graph_index \
             and try again."
        )
    })?;
    snapshot
        .into_graph()
        .map_err(|e| format!("the snapshot for '{resolved}' could not be read: {e}"))
}

fn render(base: &str, head: &str, delta: &GraphDelta, found: &DiffFindings) -> String {
    let mut out = String::new();
    out.push_str(&format!("Graph diff {base} -> {head}\n\n"));

    if delta.is_empty() {
        out.push_str("No change: the two graphs are identical.\n");
        return out;
    }

    out.push_str(&format!(
        "Summary: {} symbol(s) added, {} removed, {} renamed or moved, \
         {} signature(s) changed, {} edge(s) added, {} removed.\n",
        delta.added_symbols.len(),
        delta.removed_symbols.len(),
        delta.continuities.len(),
        delta.signature_changes.len(),
        delta.added_edges.len(),
        delta.removed_edges.len(),
    ));

    // Findings first. A broken reference is what the caller came for; a list
    // of added symbols is not.
    let broken: Vec<_> = found.of_kind(FindingKind::BrokenEdge).collect();
    if !broken.is_empty() {
        out.push_str(&format!("\nBROKEN REFERENCES ({}):\n", broken.len()));
        for finding in broken.iter().take(LIST_LIMIT) {
            out.push_str(&format!(
                "  {} was removed but is still referenced from {} place(s){}\n",
                finding.name,
                finding.referrers.len(),
                if finding.resolution == Resolution::Resolved {
                    ""
                } else {
                    " [structural — advisory only]"
                }
            ));
            for referrer in finding.referrers.iter().take(3) {
                out.push_str(&format!("      {}:{}\n", referrer.file, referrer.line));
            }
        }
        if broken.len() > LIST_LIMIT {
            out.push_str(&format!("  … and {} more\n", broken.len() - LIST_LIMIT));
        }
    }

    let api: Vec<_> = found.of_kind(FindingKind::ApiSurfaceRemoved).collect();
    if !api.is_empty() {
        out.push_str(&format!("\nAPI SURFACE REMOVED ({}):\n", api.len()));
        for finding in api.iter().take(LIST_LIMIT) {
            out.push_str(&format!(
                "  {} ({}:{})\n",
                finding.name, finding.file, finding.line
            ));
        }
        if api.len() > LIST_LIMIT {
            out.push_str(&format!("  … and {} more\n", api.len() - LIST_LIMIT));
        }
    }

    if !delta.continuities.is_empty() {
        out.push_str("\nRENAMED OR MOVED:\n");
        for continuity in delta.continuities.iter().take(LIST_LIMIT) {
            out.push_str(&format!(
                "  {} -> {} ({} -> {})\n",
                continuity.before.name,
                continuity.after.name,
                continuity.before.file,
                continuity.after.file
            ));
        }
    }

    // Reported on every diff, not only the bad ones. A caller deciding whether
    // to act on this needs to know how much of it is gate-safe, and a figure
    // that appears only sometimes is one nobody learns to read.
    out.push_str("\nCONFIDENCE:\n");
    if !delta::has_resolved_changes(delta) {
        out.push_str(
            "  Nothing here is gate-safe: every change is structural, so this diff \
             cannot support a decision to block. Treat it as advisory.\n",
        );
    } else {
        out.push_str(&format!(
            "  {} of {} finding(s) are gate-safe.\n",
            found.gate_safe().count(),
            found.findings.len()
        ));
    }
    match found.changed_file_coverage.percent() {
        Some(percent) => out.push_str(&format!(
            "  Resolution coverage over the changed files: {percent:.1}%\n"
        )),
        None => out.push_str("  No edges in the changed files.\n"),
    }

    out
}
