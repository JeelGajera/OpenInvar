//! `openinvar check` — enforce the rules a repository wrote down.
//!
//! The command a CI job runs. Everything about it is shaped by one
//! consequence: its exit status decides whether a change is allowed to land,
//! so an exit code it has not earned is worse than no gate at all.
//!
//! Three things follow from that.
//!
//! **A pass is stated, not implied.** Every rule is reported with its verdict,
//! including the ones that passed. A gate that prints nothing on success
//! teaches its readers that silence means "fine", and silence is also what a
//! misconfigured gate produces.
//!
//! **Undecided is not a pass.** A rule whose scope contains weakly resolved
//! edges is reported as undecided and does not fail the build. That is the
//! fail-open half of the contract — a Tier 2 language must not block a merge
//! on evidence it cannot stand behind — but it is printed every time, because
//! a fail-open that nobody sees is indistinguishable from a pass.
//!
//! **Only violations set a failing status**, and only at `error` severity.
//! Exit 1 means a rule was broken on resolved evidence. Exit 2 means the check
//! could not run — an unreadable rules file, no graph, a missing snapshot —
//! and is deliberately distinct, so a pipeline can tell "your change is bad"
//! from "this tool is misconfigured" rather than reading both as a veto.

use std::path::Path;

use openinvar_core::delta;
use openinvar_core::graph::InvarGraph;
use openinvar_core::rule_eval::{self, Evaluation, RuleOutcome, Verdict};
use openinvar_core::rules::{self, Rules, Severity};
use openinvar_store::RocksGraphStore;

use crate::output;

/// Version of the JSON contract. Hooks and CI read this output, so it is
/// versioned from its first release rather than when it first breaks.
const SCHEMA_VERSION: u32 = 1;

/// Exit status when every rule that could be decided was satisfied.
pub const EXIT_OK: i32 = 0;
/// Exit status when a rule was violated on resolved evidence.
pub const EXIT_VIOLATION: i32 = 1;
/// Exit status when the check could not run at all.
pub const EXIT_UNABLE: i32 = 2;

#[allow(clippy::too_many_arguments)]
pub fn run(
    path: &str,
    rules_path: Option<&str>,
    base: Option<&str>,
    head: Option<&str>,
    json: bool,
    require_rules: bool,
) -> Result<i32, Box<dyn std::error::Error>> {
    let root = super::normalize_path(
        &std::fs::canonicalize(path).map_err(|e| format!("cannot access '{}': {}", path, e))?,
    );

    let rules_file = match rules_path {
        Some(given) => std::path::PathBuf::from(given),
        None => match super::locate_config(&root) {
            Some(found) => found.path().to_path_buf(),
            // Nothing on disk. Name the supported location so the warning
            // below tells the reader where to write one.
            None => rules::config_path(&root),
        },
    };

    let Some(parsed) = load_rules(&rules_file, require_rules)? else {
        // No rules file. Nothing to enforce, and saying so plainly is the
        // point: a gate that passes because its configuration went missing
        // looks exactly like a gate that passed on the merits.
        if json {
            println!(
                r#"{{"schema_version":{SCHEMA_VERSION},"rules_file":"{}","status":"no-rules","rules":[],"summary":{{"satisfied":0,"violated":0,"inconclusive":0,"skipped":0}},"exit_code":{EXIT_OK}}}"#,
                escape(&rules_file.display().to_string())
            );
        } else {
            output::banner("check");
            output::warning(&format!(
                "No rules file at {}",
                output::file_path(&rules_file.display().to_string())
            ));
            output::dim_line("  Nothing was enforced. Write rules there, or pass --rules.");
            output::dim_line("  Use --require-rules to make a missing file a failure in CI.");
        }
        return Ok(EXIT_OK);
    };

    let graph = super::analyze::load_graph(&root)?;

    // The delta is optional, and its absence is reported rather than papered
    // over: rules that need one are skipped, never counted as satisfied.
    let (before, change) = match (base, head) {
        (Some(base), Some(head)) => {
            let (before, after) = load_pair(&root, base, head)?;
            let computed = delta::compute(&before, &after);
            (Some(before), Some(computed))
        }
        (None, None) => (None, None),
        _ => {
            return Err("--base and --head must be given together".into());
        }
    };

    let evaluated_against = before.as_ref().unwrap_or(&graph);
    let evaluation = rule_eval::evaluate(&parsed, evaluated_against, change.as_ref());

    if json {
        println!("{}", to_json(&rules_file, &evaluation));
    } else {
        report(&rules_file, &parsed, &evaluation, base, head);
    }

    Ok(if evaluation.should_fail() {
        EXIT_VIOLATION
    } else {
        EXIT_OK
    })
}

/// Read the rules file, or decide that its absence is acceptable.
///
/// `Ok(None)` means there was no file and the caller did not insist on one.
fn load_rules(
    rules_file: &Path,
    require_rules: bool,
) -> Result<Option<Rules>, Box<dyn std::error::Error>> {
    if !rules_file.exists() {
        if require_rules {
            return Err(format!(
                "no rules file at {} (--require-rules was given)",
                rules_file.display()
            )
            .into());
        }
        return Ok(None);
    }
    // A malformed rules file is fatal, never a pass. Every mistake the schema
    // can catch is caught here, before a single rule is evaluated.
    Ok(Some(rules::load(rules_file).map_err(|e| e.to_string())?))
}

fn load_pair(
    root: &Path,
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

fn severity_tag(severity: Severity) -> String {
    match severity {
        Severity::Error => output::red("[error]"),
        Severity::Warn => output::yellow("[warn]"),
    }
}

fn report(
    rules_file: &Path,
    parsed: &Rules,
    evaluation: &Evaluation,
    base: Option<&str>,
    head: Option<&str>,
) {
    output::banner("check");
    output::info(&format!(
        "{} — {} rule(s)",
        output::file_path(&rules_file.display().to_string()),
        parsed.rules.len()
    ));
    if let (Some(base), Some(head)) = (base, head) {
        output::info(&format!("against {base} → {head}"));
    }
    output::blank();

    output::section("Result");
    output::stat_highlight("Satisfied", &evaluation.count_of(Verdict::Satisfied).to_string());
    output::stat_highlight("Violated", &evaluation.count_of(Verdict::Violated).to_string());
    output::stat_highlight(
        "Undecided",
        &evaluation.count_of(Verdict::Inconclusive).to_string(),
    );
    output::stat_highlight("Skipped", &evaluation.count_of(Verdict::Skipped).to_string());

    let violated: Vec<&RuleOutcome> = evaluation
        .outcomes
        .iter()
        .filter(|o| o.verdict == Verdict::Violated)
        .collect();
    if !violated.is_empty() {
        output::section("Violations");
        for outcome in violated {
            output::stat(
                &format!("  {}", outcome.rule),
                &format!("{} {}", outcome.kind, severity_tag(outcome.severity)),
            );
            for violation in &outcome.violations {
                output::dim_line(&format!("      {}:{}", violation.file, violation.line));
                output::dim_line(&format!("        {}", violation.detail));
            }
        }
    }

    // Printed on every run, not only when it is bad news. A figure that
    // appears only sometimes is one nobody learns to read — and this is the
    // figure that says how much of the pass was actually earned.
    let undecided: Vec<&RuleOutcome> = evaluation.inconclusive().collect();
    if !undecided.is_empty() {
        output::section("Undecided");
        for outcome in &undecided {
            let detail = outcome
                .uncertainty
                .as_ref()
                .map(|u| {
                    format!(
                        "{} edge(s) in scope too weakly resolved to judge",
                        u.unresolved_edges
                    )
                })
                .unwrap_or_else(|| "could not be decided".to_string());
            output::stat(&format!("  {}", outcome.rule), &detail);
            if let Some(uncertainty) = &outcome.uncertainty {
                for file in uncertainty.files.iter().take(5) {
                    output::dim_line(&format!("      {file}"));
                }
                if uncertainty.files.len() > 5 {
                    output::dim_line(&format!(
                        "      … and {} more file(s)",
                        uncertainty.files.len() - 5
                    ));
                }
            }
        }
        output::dim_line("  These are not passes. The evidence in scope could not rule a");
        output::dim_line("  violation out, so the gate fails open rather than claim one way.");
    }

    let skipped: Vec<&RuleOutcome> = evaluation
        .outcomes
        .iter()
        .filter(|o| o.verdict == Verdict::Skipped)
        .collect();
    if !skipped.is_empty() {
        output::section("Skipped");
        for outcome in skipped {
            output::stat(
                &format!("  {}", outcome.rule),
                outcome
                    .skipped_because
                    .as_deref()
                    .unwrap_or("not evaluated"),
            );
        }
        output::dim_line("  Pass --base and --head to evaluate these against a change.");
    }

    // Satisfied rules are listed by name rather than counted, so a reader can
    // check that the rule they care about was actually among them.
    let satisfied: Vec<&RuleOutcome> = evaluation
        .outcomes
        .iter()
        .filter(|o| o.verdict == Verdict::Satisfied)
        .collect();
    if !satisfied.is_empty() {
        output::section("Satisfied");
        for outcome in satisfied {
            output::stat(&format!("  {}", outcome.rule), outcome.kind);
        }
    }

    // The closing line is the one most readers act on, so it must never
    // overstate. A warn-severity breach does not fail the build but is still a
    // breach: reporting "all rules satisfied" because the exit status happens
    // to be zero would be the exact dishonesty this command exists to avoid.
    output::blank();
    let violated_count = evaluation.count_of(Verdict::Violated);
    let satisfied_count = evaluation.count_of(Verdict::Satisfied);
    let skipped_count = evaluation.count_of(Verdict::Skipped);
    let blocking = evaluation.blocking().count();

    if parsed.rules.is_empty() {
        output::warning("The rules file defines no rules. Nothing was enforced.");
        return;
    }

    if blocking > 0 {
        output::error(&format!("{blocking} rule(s) violated."));
    } else if violated_count > 0 {
        output::warning(&format!(
            "{violated_count} rule(s) violated at warn severity — exit status unaffected."
        ));
    } else if undecided.is_empty() && skipped_count == 0 && satisfied_count > 0 {
        // Every rule was examined in full and none broke. The only case that
        // earns this sentence.
        output::success("All rules satisfied.");
    }

    // Said after the verdict, and unconditionally, because these are the rules
    // whose silence a reader would otherwise count as agreement.
    if !undecided.is_empty() {
        output::warning(&format!(
            "{} rule(s) could not be decided.",
            undecided.len()
        ));
    }
    if skipped_count > 0 {
        output::warning(&format!("{skipped_count} rule(s) were not evaluated."));
    }
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

fn outcome_json(outcome: &RuleOutcome) -> String {
    let violations: Vec<String> = outcome
        .violations
        .iter()
        .map(|v| {
            format!(
                r#"{{"file":"{}","line":{},"symbol":"{}","detail":"{}"}}"#,
                escape(&v.file),
                v.line,
                escape(&v.symbol),
                escape(&v.detail)
            )
        })
        .collect();

    let uncertainty = match &outcome.uncertainty {
        Some(u) => {
            let files: Vec<String> = u
                .files
                .iter()
                .map(|f| format!(r#""{}""#, escape(f)))
                .collect();
            format!(
                r#"{{"unresolved_edges":{},"files":[{}]}}"#,
                u.unresolved_edges,
                files.join(",")
            )
        }
        None => "null".to_string(),
    };

    let skipped = match &outcome.skipped_because {
        Some(reason) => format!(r#""{}""#, escape(reason)),
        None => "null".to_string(),
    };

    format!(
        r#"{{"rule":"{}","kind":"{}","severity":"{}","verdict":"{}","violations":[{}],"uncertainty":{},"skipped_because":{}}}"#,
        escape(&outcome.rule),
        outcome.kind,
        outcome.severity.as_str(),
        outcome.verdict.as_str(),
        violations.join(","),
        uncertainty,
        skipped
    )
}

fn to_json(rules_file: &Path, evaluation: &Evaluation) -> String {
    let outcomes: Vec<String> = evaluation.outcomes.iter().map(outcome_json).collect();
    let exit_code = if evaluation.should_fail() {
        EXIT_VIOLATION
    } else {
        EXIT_OK
    };
    format!(
        r#"{{"schema_version":{},"rules_file":"{}","status":"evaluated","rules":[{}],"summary":{{"satisfied":{},"violated":{},"inconclusive":{},"skipped":{}}},"exit_code":{}}}"#,
        SCHEMA_VERSION,
        escape(&rules_file.display().to_string()),
        outcomes.join(","),
        evaluation.count_of(Verdict::Satisfied),
        evaluation.count_of(Verdict::Violated),
        evaluation.count_of(Verdict::Inconclusive),
        evaluation.count_of(Verdict::Skipped),
        exit_code
    )
}
