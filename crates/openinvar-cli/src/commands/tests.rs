//! `openinvar tests` — which tests exercise a change.
//!
//! The verify loop. An agent today either runs the whole suite, which is slow
//! enough that it skips it, or runs nothing, which is unsafe. This makes the
//! middle option cheap enough to actually take.
//!
//! What makes that dangerous, and what this command is built around: the
//! answer licenses an *omission*. Naming the tests that cover a change is a
//! claim that the ones left out cannot fail, and OpenInvar's graph is knowingly
//! incomplete — structural regions record no cross-file references, and test
//! detection is by file convention.
//!
//! So the selection and the confidence in it are reported as two separate
//! things, and the exit status carries the second. A caller that runs only
//! these tests after a non-zero status is making the omission claim on its own
//! authority.

use std::collections::BTreeSet;

use openinvar_core::delta;
use openinvar_core::graph::InvarGraph;
use openinvar_core::ir::Resolution;
use openinvar_core::test_impact::{self, TestSelection};
use openinvar_store::RocksGraphStore;

use crate::output;

/// Version of the JSON contract, read by hooks and CI.
const SCHEMA_VERSION: u32 = 1;

/// Selection is complete enough to run instead of the suite.
pub const EXIT_COMPLETE: i32 = 0;
/// Tests were found, but something could be missing. Not a failure — a caveat
/// with an exit code, so a script can tell the two apart without parsing prose.
pub const EXIT_INCOMPLETE: i32 = 3;
/// The question could not be answered at all.
pub const EXIT_UNABLE: i32 = 2;

#[allow(clippy::too_many_arguments)]
pub fn run(
    symbol: Option<&str>,
    path: &str,
    base: Option<&str>,
    head: Option<&str>,
    diff: bool,
    depth: usize,
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

    let (graph, changed, subject) = if diff {
        let base = base.unwrap_or("HEAD");
        let head = head.unwrap_or("worktree");
        let (before, after) = load_pair(&root, base, head)?;
        let computed = delta::compute(&before, &after);
        // Evaluated against the graph the change started from: a removed
        // symbol has no node in the after-graph, so the tests that covered it
        // would be invisible in exactly the case that matters most.
        let changed = changed_symbol_ids(&computed);
        (before, changed, format!("{base} → {head}"))
    } else {
        let Some(symbol) = symbol else {
            return Err(
                "give a symbol, or --diff to select tests for a recorded change".into(),
            );
        };
        let graph = super::analyze::load_graph(&root)?;
        let ids = ids_for_symbol(&graph, symbol)?;
        (graph, ids, symbol.to_string())
    };

    let selection = test_impact::select(&graph, &changed, depth, floor);

    if json {
        println!("{}", to_json(&subject, &changed, &selection));
    } else {
        report(&subject, &changed, &selection);
    }

    Ok(if selection.is_trustworthy() {
        EXIT_COMPLETE
    } else {
        EXIT_INCOMPLETE
    })
}

/// Every symbol a delta touched.
///
/// Includes both sides of a rename and of a signature change: the test that
/// covered the old name is the test that should run against the new one.
fn changed_symbol_ids(delta: &delta::GraphDelta) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    for symbol in delta.added_symbols.iter().chain(delta.removed_symbols.iter()) {
        out.insert(symbol.id.clone());
    }
    for continuity in &delta.continuities {
        out.insert(continuity.before.id.clone());
        out.insert(continuity.after.id.clone());
    }
    for change in &delta.signature_changes {
        out.insert(change.before.id.clone());
        out.insert(change.after.id.clone());
    }
    // An edge that appeared or vanished changed the behaviour of the symbol it
    // leaves, even when that symbol's own definition is untouched.
    for edge in delta.added_edges.iter().chain(delta.removed_edges.iter()) {
        out.insert(edge.from.clone());
    }
    out
}

fn ids_for_symbol(graph: &InvarGraph, symbol: &str) -> Result<BTreeSet<String>, String> {
    // Every symbol of that name. An ambiguous name is not an error here: a
    // caller asking which tests cover `encode` wants all of them, and picking
    // one arbitrarily would silently answer a narrower question.
    let ids: BTreeSet<String> = graph
        .name_index
        .get(symbol)
        .map(|ids| ids.iter().cloned().collect())
        .unwrap_or_default();

    if ids.is_empty() {
        return Err(format!(
            "no symbol named '{symbol}' in the graph. Run: openinvar analyze . — if it is new."
        ));
    }
    Ok(ids)
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

fn load_one(
    store: &RocksGraphStore,
    resolved: &str,
    given: &str,
) -> Result<InvarGraph, String> {
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

fn report(subject: &str, changed: &BTreeSet<String>, selection: &TestSelection) {
    output::banner("tests");
    output::info(subject);
    output::blank();

    output::section("Summary");
    output::stat_highlight("Symbols changed", &changed.len().to_string());
    output::stat_highlight("Covering tests", &selection.tests.len().to_string());
    output::stat_highlight("Test files", &selection.files().len().to_string());
    output::stat_highlight(
        "Tests modified by this change",
        &selection.modified_tests.len().to_string(),
    );

    if !selection.tests.is_empty() {
        output::section("Run these");
        for file in selection.files() {
            output::stat(&format!("  {file}"), "");
        }
    }

    if !selection.modified_tests.is_empty() {
        // Reported apart from coverage on purpose: a test the change itself
        // edited is the thing whose behaviour changed, not evidence that
        // anything still works.
        output::section("Tests this change modified");
        for test in &selection.modified_tests {
            output::stat(&format!("  {}", test.file), "changed by this diff");
        }
    }

    // Always printed, not only when it is bad news. This is the figure that
    // says whether the list above may be run *instead of* the suite, and a
    // number that appears only sometimes is one nobody learns to read.
    output::section("Confidence");
    if selection.is_trustworthy() {
        output::success("Every changed symbol is reached by a resolved test edge.");
        output::dim_line("  This selection may be run in place of the full suite.");
    } else {
        output::warning("This selection may be incomplete.");
        for gap in &selection.gaps {
            output::dim_line(&format!("  - {}", gap.describe()));
        }
        output::blank();
        output::dim_line("  Run the full suite. Naming a subset is a claim that the tests");
        output::dim_line("  left out cannot fail, and that claim is not earned here.");
    }
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

fn test_json(test: &openinvar_core::test_impact::CoveringTest) -> String {
    let covers: Vec<String> = test
        .covers
        .iter()
        .map(|c| format!(r#""{}""#, escape(c)))
        .collect();
    format!(
        r#"{{"symbol":"{}","file":"{}","gate_safe":{},"covers":[{}]}}"#,
        escape(&test.symbol),
        escape(&test.file),
        test.gate_safe,
        covers.join(",")
    )
}

fn to_json(subject: &str, changed: &BTreeSet<String>, selection: &TestSelection) -> String {
    let tests: Vec<String> = selection.tests.iter().map(test_json).collect();
    let modified: Vec<String> = selection.modified_tests.iter().map(test_json).collect();
    let files: Vec<String> = selection
        .files()
        .iter()
        .map(|f| format!(r#""{}""#, escape(f)))
        .collect();
    let uncovered: Vec<String> = selection
        .uncovered
        .iter()
        .map(|u| format!(r#""{}""#, escape(u)))
        .collect();
    let gaps: Vec<String> = selection
        .gaps
        .iter()
        .map(|g| format!(r#""{}""#, escape(&g.describe())))
        .collect();

    format!(
        r#"{{"schema_version":{},"subject":"{}","symbols_changed":{},"files":[{}],"tests":[{}],"modified_tests":[{}],"uncovered":[{}],"gaps":[{}],"complete":{},"exit_code":{}}}"#,
        SCHEMA_VERSION,
        escape(subject),
        changed.len(),
        files.join(","),
        tests.join(","),
        modified.join(","),
        uncovered.join(","),
        gaps.join(","),
        selection.is_trustworthy(),
        if selection.is_trustworthy() {
            EXIT_COMPLETE
        } else {
            EXIT_INCOMPLETE
        }
    )
}
