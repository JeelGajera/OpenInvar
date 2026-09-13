//! MCP tool: check_rules
//!
//! Evaluate `.openinvar/rules.toml` and report what the change breaks.
//!
//! The agent-facing half of `openinvar check`. It reports the same four verdicts
//! and takes the same care with them: a rule too weakly resolved to judge is
//! reported as undecided, a rule needing a change that was not supplied is
//! reported as skipped, and neither is presented as a pass.
//!
//! That care matters more here than at the command line. A person reading a
//! terminal notices a blank section; a model reading a summary that says
//! "no violations" will act on it. So this tool never emits that sentence
//! unless every rule was examined in full.

use std::path::Path;

use schemars::JsonSchema;
use serde::Deserialize;

use openinvar_core::delta;
use openinvar_core::graph::InvarGraph;
use openinvar_core::rule_eval::{self, Evaluation, Verdict};
use openinvar_core::rules;
use openinvar_store::RocksGraphStore;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckRulesParams {
    /// Optional: evaluate change-sensitive rules against this revision as
    /// "before". Must be given together with `head`, and both must already be
    /// recorded with `openinvar analyze --snapshot`. Omit both to evaluate only
    /// the rules that need a single graph.
    pub base: Option<String>,
    /// Optional: the revision to compare to. 'worktree' is the working tree
    /// including uncommitted edits.
    pub head: Option<String>,
}

pub fn execute(
    repo_root: &Path,
    graph: &InvarGraph,
    params: CheckRulesParams,
) -> Result<String, String> {
    let rules_file = rules::default_path(repo_root);
    if !rules_file.exists() {
        // Not an error, and not silence either. A caller told "no violations"
        // by a repository that has written no rules would draw exactly the
        // wrong conclusion.
        return Ok(format!(
            "No rules file at {}. Nothing was enforced — this is not a pass. \
             Write rules there to have this tool check anything.",
            rules_file.display()
        ));
    }

    let parsed = rules::load(&rules_file).map_err(|e| {
        format!("The rules file could not be read, so no rule was checked: {e}")
    })?;

    let (before, change) = match (params.base.as_deref(), params.head.as_deref()) {
        (Some(base), Some(head)) => {
            let base_rev = openinvar_store::revision::resolve(repo_root, base)?;
            let head_rev = openinvar_store::revision::resolve(repo_root, head)?;
            let store = RocksGraphStore::open(&repo_root.join(".openinvar").join("graph.db"))
                .map_err(|e| format!("failed to open the graph store: {e}"))?;
            let before = load(&store, &base_rev, base)?;
            let after = load(&store, &head_rev, head)?;
            let computed = delta::compute(&before, &after);
            (Some(before), Some(computed))
        }
        (None, None) => (None, None),
        _ => return Err("base and head must be given together".to_string()),
    };

    let evaluated_against = before.as_ref().unwrap_or(graph);
    let evaluation = rule_eval::evaluate(&parsed, evaluated_against, change.as_ref());

    Ok(render(&evaluation))
}

fn load(store: &RocksGraphStore, resolved: &str, given: &str) -> Result<InvarGraph, String> {
    let snapshot = store.load_revision(resolved).map_err(|_| {
        format!(
            "No snapshot recorded for {given}. Record one first with: \
             openinvar analyze . --snapshot {given}"
        )
    })?;
    snapshot
        .into_graph()
        .map_err(|e| format!("the snapshot for '{resolved}' could not be read: {e}"))
}

fn render(evaluation: &Evaluation) -> String {
    let mut out = String::new();
    let violated = evaluation.count_of(Verdict::Violated);
    let undecided = evaluation.count_of(Verdict::Inconclusive);
    let skipped = evaluation.count_of(Verdict::Skipped);
    let satisfied = evaluation.count_of(Verdict::Satisfied);

    out.push_str(&format!(
        "Rules: {satisfied} satisfied, {violated} violated, \
         {undecided} undecided, {skipped} skipped.\n"
    ));

    for outcome in &evaluation.outcomes {
        if outcome.verdict != Verdict::Violated {
            continue;
        }
        out.push_str(&format!(
            "\nVIOLATED [{}] {} ({}):\n",
            outcome.severity.as_str(),
            outcome.rule,
            outcome.kind
        ));
        for violation in outcome.violations.iter().take(10) {
            out.push_str(&format!(
                "  {}:{} — {}\n",
                violation.file, violation.line, violation.detail
            ));
        }
        if outcome.violations.len() > 10 {
            out.push_str(&format!(
                "  … and {} more\n",
                outcome.violations.len() - 10
            ));
        }
    }

    for outcome in evaluation.inconclusive() {
        let detail = outcome
            .uncertainty
            .as_ref()
            .map(|u| format!("{} edge(s) in scope too weakly resolved to judge", u.unresolved_edges))
            .unwrap_or_else(|| "could not be decided".to_string());
        out.push_str(&format!("\nUNDECIDED {}: {detail}\n", outcome.rule));
    }

    for outcome in evaluation.outcomes.iter().filter(|o| o.verdict == Verdict::Skipped) {
        out.push_str(&format!(
            "\nSKIPPED {}: {}\n",
            outcome.rule,
            outcome.skipped_because.as_deref().unwrap_or("not evaluated")
        ));
    }

    out.push('\n');
    if evaluation.should_fail() {
        out.push_str(&format!(
            "This change violates {} rule(s) at error severity. \
             Fix them before committing.\n",
            evaluation.blocking().count()
        ));
    } else if violated > 0 {
        out.push_str(
            "Violations were found at warn severity only. They do not block, \
             but they are real.\n",
        );
    } else if undecided == 0 && skipped == 0 && satisfied > 0 {
        // The only case that earns this sentence.
        out.push_str("Every rule was examined in full and none was violated.\n");
    } else if satisfied == 0 && undecided == 0 && violated == 0 {
        out.push_str(
            "Nothing was actually evaluated. This is not a pass — pass base and \
             head to check rules that need a change.\n",
        );
    } else {
        out.push_str(
            "No rule was violated, but not every rule could be decided. \
             Treat this as an incomplete check rather than a clean one.\n",
        );
    }

    out
}
