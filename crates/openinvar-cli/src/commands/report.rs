//! `openinvar report` — one markdown report for a pull request.
//!
//! The Action posts exactly one comment, so exactly one command produces it.
//! Concatenating `diff` and `check` output would give the reader two headings,
//! two verdicts and no statement of which one decides the merge.
//!
//! Exit status matches `check`, because the same thing is being decided:
//! 0 when nothing was violated, 1 when a rule was broken on resolved evidence,
//! 2 when the report could not be produced at all. A CI job reads the status;
//! a person reads the comment; neither should have to consult the other to
//! find out what happened.


use openinvar_core::delta;
use openinvar_core::findings;
use openinvar_core::graph::InvarGraph;
use openinvar_core::rule_eval::{self, Evaluation};
use openinvar_core::rules;
use openinvar_store::RocksGraphStore;

use super::check::{EXIT_OK, EXIT_VIOLATION};
use super::markdown;

pub fn run(
    path: &str,
    base: &str,
    head: &str,
    rules_path: Option<&str>,
    require_rules: bool,
) -> Result<i32, Box<dyn std::error::Error>> {
    let root = super::normalize_path(
        &std::fs::canonicalize(path).map_err(|e| format!("cannot access '{}': {}", path, e))?,
    );

    let base_rev = openinvar_store::revision::resolve(&root, base)?;
    let head_rev = openinvar_store::revision::resolve(&root, head)?;

    let store = RocksGraphStore::open(&super::db_path(&root))
        .map_err(|e| format!("failed to open store: {e}"))?;
    let before = load(&store, &base_rev, base)?;
    let after = load(&store, &head_rev, head)?;

    let computed = delta::compute(&before, &after);
    let found = findings::derive(&before, &after, &computed);

    // Rules are optional. A repository without them still gets a useful diff
    // report, and saying nothing about rules is honest when none were written.
    let rules_file = match rules_path {
        Some(given) => std::path::PathBuf::from(given),
        None => match super::locate_config(&root) {
            Some(found) => found.path().to_path_buf(),
            None => rules::config_path(&root),
        },
    };
    let evaluation: Option<Evaluation> = if rules_file.exists() {
        let parsed = rules::load(&rules_file).map_err(|e| e.to_string())?;
        Some(rule_eval::evaluate(&parsed, &before, Some(&computed)))
    } else if require_rules {
        // The caller said a rules file must exist. Producing a clean report
        // having enforced nothing is precisely what they asked to prevent.
        return Err(format!(
            "no rules file at {} (--require-rules was given)",
            rules_file.display()
        )
        .into());
    } else {
        None
    };

    println!(
        "{}",
        markdown::report(
            &base_rev,
            &head_rev,
            Some(&computed),
            Some(&found),
            evaluation.as_ref(),
        )
    );

    Ok(match &evaluation {
        Some(evaluation) if evaluation.should_fail() => EXIT_VIOLATION,
        _ => EXIT_OK,
    })
}

fn load(store: &RocksGraphStore, resolved: &str, given: &str) -> Result<InvarGraph, String> {
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
