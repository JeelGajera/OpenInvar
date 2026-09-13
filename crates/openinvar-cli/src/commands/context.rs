//! `openinvar context` — the minimal working set for a symbol or a change.
//!
//! The cheapest useful answer to "what am I about to touch, and what touches
//! it". An agent without this reads whole files; this emits the neighbourhood
//! as one line per symbol.
//!
//! The token figure is an estimate and is labelled as one everywhere it
//! appears. OpenInvar vendors no tokenizer and will not: a tokenizer is
//! model-specific, and a figure that moved with somebody's model would not be
//! reproducible. The comparison against reading the files whole is measured
//! from disk rather than asserted.

use std::collections::BTreeSet;

use openinvar_core::context::{self, Role, WholeFileCost, WorkingSet};
use openinvar_core::delta;
use openinvar_core::graph::InvarGraph;
use openinvar_core::ir::Resolution;
use openinvar_store::RocksGraphStore;

use crate::output;

const SCHEMA_VERSION: u32 = 1;

pub const EXIT_OK: i32 = 0;
pub const EXIT_UNABLE: i32 = 2;

#[allow(clippy::too_many_arguments)]
pub fn run(
    symbol: Option<&str>,
    path: &str,
    diff: bool,
    base: Option<&str>,
    head: Option<&str>,
    depth: usize,
    budget: Option<usize>,
    min_confidence: &str,
    json: bool,
) -> Result<i32, Box<dyn std::error::Error>> {
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

    let (graph, subjects, label) = if diff {
        let base = base.unwrap_or("HEAD");
        let head = head.unwrap_or("worktree");
        let (before, after) = load_pair(&root, base, head)?;
        let computed = delta::compute(&before, &after);
        let ids = changed_ids(&computed);
        // The after-graph: an agent orienting on a change works on the tree as
        // it now is, and a symbol the change added exists only there.
        (after, ids, format!("{base} → {head}"))
    } else {
        let Some(symbol) = symbol else {
            return Err("give a symbol, or --diff to orient on a change".into());
        };
        let graph = super::analyze::load_graph(&root)?;
        let ids: BTreeSet<String> = graph
            .name_index
            .get(symbol)
            .map(|ids| ids.iter().cloned().collect())
            .unwrap_or_default();
        if ids.is_empty() {
            return Err(format!("no symbol named '{symbol}' in the graph").into());
        }
        (graph, ids, symbol.to_string())
    };

    let set = context::build(&graph, &subjects, depth, budget, floor);
    let whole = context::whole_file_tokens(&root, &set.files);

    if json {
        println!("{}", to_json(&label, &set, &whole));
    } else {
        report(&label, &set, &whole);
    }
    Ok(EXIT_OK)
}

fn changed_ids(delta: &delta::GraphDelta) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for symbol in delta.added_symbols.iter().chain(delta.removed_symbols.iter()) {
        out.insert(symbol.id.clone());
    }
    for c in &delta.continuities {
        out.insert(c.after.id.clone());
    }
    for c in &delta.signature_changes {
        out.insert(c.after.id.clone());
    }
    for e in delta.added_edges.iter().chain(delta.removed_edges.iter()) {
        out.insert(e.from.clone());
    }
    out
}

fn load_pair(
    root: &std::path::Path,
    base: &str,
    head: &str,
) -> Result<(InvarGraph, InvarGraph), Box<dyn std::error::Error>> {
    let base_rev = openinvar_store::revision::resolve(root, base)?;
    let head_rev = openinvar_store::revision::resolve(root, head)?;
    let store = RocksGraphStore::open(&super::db_path(root))
        .map_err(|e| format!("failed to open store: {e}"))?;
    Ok((
        load_one(&store, &base_rev, base)?,
        load_one(&store, &head_rev, head)?,
    ))
}

fn load_one(store: &RocksGraphStore, resolved: &str, given: &str) -> Result<InvarGraph, String> {
    let snapshot = store.load_revision(resolved).map_err(|_| {
        format!(
            "no snapshot recorded for {given}. \
             Record one with: openinvar analyze . --snapshot {given}"
        )
    })?;
    snapshot
        .into_graph()
        .map_err(|e| format!("snapshot for '{resolved}' could not be read: {e}"))
}

fn report(label: &str, set: &WorkingSet, whole: &WholeFileCost) {
    output::banner("context");
    output::info(label);
    output::blank();

    // The working set itself, on stdout and unadorned: this is what gets
    // pasted into a prompt, so it must survive a copy without the banner.
    for role in [Role::Subject, Role::Dependency, Role::Dependent] {
        let group: Vec<_> = set.entries.iter().filter(|e| e.role == role).collect();
        if group.is_empty() {
            continue;
        }
        output::section(match role {
            Role::Subject => "Subject",
            Role::Dependency => "Depends on",
            Role::Dependent => "Depended on by",
        });
        for entry in group {
            println!("  {}", entry.render());
        }
    }

    output::section("Cost");
    output::stat_highlight("Symbols", &set.entries.len().to_string());
    output::stat_highlight("Files touched", &set.files.len().to_string());
    output::stat_highlight(
        "Estimated tokens",
        &format!("~{}", set.estimated_tokens),
    );

    if whole.files_read > 0 {
        output::stat(
            "Reading those files whole",
            &format!("~{} estimated tokens", whole.tokens),
        );
        // Stated as a ratio only when there is one worth stating. A working
        // set no smaller than the files is a working set not worth building,
        // and saying so is cheaper than letting someone discover it.
        if set.estimated_tokens > 0 && whole.tokens > set.estimated_tokens {
            let ratio = whole.tokens as f64 / set.estimated_tokens as f64;
            output::stat("Ratio", &format!("{ratio:.1}x smaller"));
        } else {
            output::warning("This working set is not smaller than reading the files whole.");
            output::dim_line("  For a neighbourhood this small, reading the files is simpler.");
        }
    } else {
        output::stat("Reading those files whole", "no readable file to compare against");
    }
    if whole.files_unreadable > 0 {
        // Named rather than folded in: a file counted as zero would make the
        // saving look larger than it is, and the saving is the whole claim.
        output::dim_line(&format!(
            "  {} file(s) could not be read and are excluded from the comparison.",
            whole.files_unreadable
        ));
    }

    output::dim_line("  Token figures are byte-based estimates, not a tokenizer's count.");

    if set.omitted > 0 {
        output::blank();
        output::warning(&format!(
            "{} symbol(s) omitted to stay inside the budget.",
            set.omitted
        ));
        output::dim_line("  The outermost hops were dropped first. Raise --budget to see them.");
    }
    if !set.gate_safe {
        output::blank();
        output::warning("Some edges here are structural — this neighbourhood may be incomplete.");
    }
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

fn to_json(label: &str, set: &WorkingSet, whole: &WholeFileCost) -> String {
    let entries: Vec<String> = set
        .entries
        .iter()
        .map(|e| {
            format!(
                r#"{{"role":"{}","hop":{},"file":"{}","line":{},"name":"{}","kind":"{}","signature":{}}}"#,
                e.role.as_str(),
                e.hop,
                escape(&e.file),
                e.line,
                escape(&e.name),
                e.kind,
                match &e.signature {
                    Some(s) => format!(r#""{}""#, escape(s)),
                    None => "null".to_string(),
                }
            )
        })
        .collect();

    format!(
        r#"{{"schema_version":{},"subject":"{}","entries":[{}],"omitted":{},"estimated_tokens":{},"whole_file_estimated_tokens":{},"files":{},"gate_safe":{},"estimate_method":"bytes/4, not a tokenizer","exit_code":{}}}"#,
        SCHEMA_VERSION,
        escape(label),
        entries.join(","),
        set.omitted,
        set.estimated_tokens,
        if whole.files_read > 0 {
            whole.tokens.to_string()
        } else {
            "null".to_string()
        },
        set.files.len(),
        set.gate_safe,
        EXIT_OK
    )
}
