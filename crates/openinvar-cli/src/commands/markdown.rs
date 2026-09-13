//! Markdown rendering for `diff` and `check`.
//!
//! This is the format most people will ever see OpenInvar in: a comment on a
//! pull request, read once, in a hurry, by someone deciding whether to click
//! merge. It is a product surface, not a serialisation of internal state.
//!
//! Three rules follow from who reads it.
//!
//! **The verdict is the first line.** A reader who stops after one line must
//! still have the answer. Everything below it is evidence for a decision
//! already stated.
//!
//! **Findings before counts.** A broken reference is what the reader came for.
//! "14 symbols added" is not, and putting it first buries the finding under
//! arithmetic.
//!
//! **Uncertainty is never omitted for tidiness.** A rule that could not be
//! decided, or a diff with no gate-safe evidence in it, says so in the comment
//! rather than only in the exit status. The comment is what gets read.

use openinvar_core::delta::{self, GraphDelta};
use openinvar_core::findings::{DiffFindings, FindingKind};
use openinvar_core::ir::Resolution;
use openinvar_core::rule_eval::{Evaluation, Verdict};

/// Marks the comment as OpenInvar's so a later run updates it instead of
/// posting a second one. A pull request with nine identical bot comments is
/// one where nobody reads the tenth.
pub const COMMENT_MARKER: &str = "<!-- openinvar-report -->";

/// How many rows of one kind are listed before the rest are summarised.
///
/// A comment long enough to collapse is a comment nobody expands.
const LIST_LIMIT: usize = 10;

fn escape(value: &str) -> String {
    value.replace('|', "\\|").replace('`', "'")
}

/// One row of the summary table.
fn row(label: &str, value: impl std::fmt::Display) -> String {
    format!("| {label} | {value} |\n")
}

/// The whole report: rules first, because that is the part that blocks.
pub fn report(
    base: &str,
    head: &str,
    delta: Option<&GraphDelta>,
    found: Option<&DiffFindings>,
    evaluation: Option<&Evaluation>,
) -> String {
    let mut out = String::from(COMMENT_MARKER);
    out.push_str("\n## OpenInvar\n\n");
    out.push_str(&headline(delta, found, evaluation));
    out.push('\n');

    match evaluation {
        Some(evaluation) => out.push_str(&rules_section(evaluation)),
        // Said explicitly. A comment silent about rules reads as a comment
        // whose rules passed.
        None => out.push_str(
            "\n### Rules\n\nNo `.openinvar/rules.toml` in this repository, so no rule was \
             enforced. This is not a passing rule check — it is the absence of one.\n",
        ),
    }
    if let (Some(delta), Some(found)) = (delta, found) {
        out.push_str(&diff_section(base, head, delta, found));
    }

    out.push_str("\n<sub>Deterministic analysis — no model was consulted. ");
    out.push_str("Structural results are advisory: they are matched within one file ");
    out.push_str("and cannot see across files.</sub>\n");
    out
}

/// The one line a reader who stops reading must still get right.
fn headline(
    delta: Option<&GraphDelta>,
    found: Option<&DiffFindings>,
    evaluation: Option<&Evaluation>,
) -> String {
    if let Some(evaluation) = evaluation {
        let blocking = evaluation.blocking().count();
        if blocking > 0 {
            return format!("**{blocking} rule(s) violated.** This change is blocked.\n");
        }
        let violated = evaluation.count_of(Verdict::Violated);
        if violated > 0 {
            return format!(
                "**{violated} rule(s) violated at warn severity.** Not blocking, but real.\n"
            );
        }
    }

    let broken = found
        .map(|f| f.of_kind(FindingKind::BrokenEdge).count())
        .unwrap_or(0);
    if broken > 0 {
        return format!(
            "**{broken} broken reference(s).** Symbols were removed while something \
             still refers to them.\n"
        );
    }

    // Only now can this be said, and only with the qualifications intact.
    let undecided = evaluation
        .map(|e| e.count_of(Verdict::Inconclusive) + e.count_of(Verdict::Skipped))
        .unwrap_or(0);
    if undecided > 0 {
        return format!(
            "No violation found, but **{undecided} rule(s) could not be decided**. \
             Treat this as an incomplete check rather than a clean one.\n"
        );
    }
    if delta.is_some_and(|d| d.is_empty()) {
        return "No graph change.\n".to_string();
    }
    if evaluation.is_some_and(|e| e.count_of(Verdict::Satisfied) > 0) {
        return "**No problems found.** Every rule was examined in full.\n".to_string();
    }
    // No rules were evaluated, so the only claim this report can make is about
    // references. "No problems found" would let a reader conclude that rules
    // passed, when the repository has none — the same unearned pass `check`
    // refuses to report when the rules file is missing.
    "**No broken references.** No rules were evaluated.\n".to_string()
}

fn verdict_icon(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Satisfied => "pass",
        Verdict::Violated => "**FAIL**",
        Verdict::Inconclusive => "undecided",
        Verdict::Skipped => "skipped",
    }
}

fn rules_section(evaluation: &Evaluation) -> String {
    if evaluation.outcomes.is_empty() {
        return String::new();
    }

    let mut out = String::from("\n### Rules\n\n");
    out.push_str("| Rule | Kind | Verdict |\n|---|---|---|\n");
    for outcome in &evaluation.outcomes {
        out.push_str(&format!(
            "| {} | {} | {} |\n",
            escape(&outcome.rule),
            outcome.kind,
            verdict_icon(outcome.verdict)
        ));
    }

    for outcome in evaluation
        .outcomes
        .iter()
        .filter(|o| o.verdict == Verdict::Violated)
    {
        out.push_str(&format!(
            "\n**{}** ({}, {}):\n\n",
            escape(&outcome.rule),
            outcome.kind,
            outcome.severity.as_str()
        ));
        for violation in outcome.violations.iter().take(LIST_LIMIT) {
            out.push_str(&format!(
                "- `{}:{}` — {}\n",
                violation.file,
                violation.line,
                escape(&violation.detail)
            ));
        }
        if outcome.violations.len() > LIST_LIMIT {
            out.push_str(&format!(
                "- … and {} more\n",
                outcome.violations.len() - LIST_LIMIT
            ));
        }
    }

    // Said in the comment, not only in the exit status. The comment is what
    // gets read, and a rule nobody could decide is not a rule that passed.
    for outcome in evaluation.inconclusive() {
        let edges = outcome
            .uncertainty
            .as_ref()
            .map(|u| u.unresolved_edges)
            .unwrap_or(0);
        out.push_str(&format!(
            "\n**{}** could not be decided: {edges} edge(s) in scope were too weakly \
             resolved to rule a violation out.\n",
            escape(&outcome.rule)
        ));
    }

    out
}

fn diff_section(base: &str, head: &str, delta: &GraphDelta, found: &DiffFindings) -> String {
    let mut out = format!("\n### Changes ({} → {})\n\n", short(base), short(head));

    if delta.is_empty() {
        out.push_str("The two graphs are identical.\n");
        return out;
    }

    // Findings before counts: a broken reference is what the reader came for.
    let broken: Vec<_> = found.of_kind(FindingKind::BrokenEdge).collect();
    if !broken.is_empty() {
        out.push_str("**Broken references**\n\n");
        for finding in broken.iter().take(LIST_LIMIT) {
            out.push_str(&format!(
                "- `{}` removed, still referenced from {} place(s){}\n",
                escape(&finding.name),
                finding.referrers.len(),
                if finding.resolution == Resolution::Resolved {
                    ""
                } else {
                    " _(structural — advisory)_"
                }
            ));
            for referrer in finding.referrers.iter().take(3) {
                out.push_str(&format!("  - `{}:{}`\n", referrer.file, referrer.line));
            }
        }
        if broken.len() > LIST_LIMIT {
            out.push_str(&format!("- … and {} more\n", broken.len() - LIST_LIMIT));
        }
        out.push('\n');
    }

    let api: Vec<_> = found.of_kind(FindingKind::ApiSurfaceRemoved).collect();
    if !api.is_empty() {
        out.push_str("**API surface removed**\n\n");
        for finding in api.iter().take(LIST_LIMIT) {
            out.push_str(&format!(
                "- `{}` (`{}:{}`)\n",
                escape(&finding.name),
                finding.file,
                finding.line
            ));
        }
        if api.len() > LIST_LIMIT {
            out.push_str(&format!("- … and {} more\n", api.len() - LIST_LIMIT));
        }
        out.push('\n');
    }

    if !delta.continuities.is_empty() {
        // Separate from add/remove because that is the point of pairing them:
        // a rename is one change, not a destruction and a creation.
        out.push_str("**Renamed or moved**\n\n");
        for continuity in delta.continuities.iter().take(LIST_LIMIT) {
            out.push_str(&format!(
                "- `{}` → `{}`\n",
                escape(&continuity.before.name),
                escape(&continuity.after.name)
            ));
        }
        out.push('\n');
    }

    out.push_str("<details><summary>Counts</summary>\n\n");
    out.push_str("| | |\n|---|---|\n");
    out.push_str(&row("Symbols added", delta.added_symbols.len()));
    out.push_str(&row("Symbols removed", delta.removed_symbols.len()));
    out.push_str(&row("Renamed or moved", delta.continuities.len()));
    out.push_str(&row("Signatures changed", delta.signature_changes.len()));
    out.push_str(&row("Edges added", delta.added_edges.len()));
    out.push_str(&row("Edges removed", delta.removed_edges.len()));
    out.push_str("\n</details>\n");

    // Never omitted for tidiness.
    if !delta::has_resolved_changes(delta) && !delta.is_empty() {
        out.push_str(
            "\n> Every change here is structural. Nothing in this diff is gate-safe, \
             so it cannot support a decision to block.\n",
        );
    } else if let Some(percent) = found.changed_file_coverage.percent() {
        out.push_str(&format!(
            "\n<sub>Resolution coverage over the changed files: {percent:.1}%.</sub>\n"
        ));
    }

    out
}

fn short(revision: &str) -> String {
    if revision.len() == 40 && revision.chars().all(|c| c.is_ascii_hexdigit()) {
        revision[..8].to_string()
    } else {
        revision.to_string()
    }
}

#[cfg(test)]
mod tests {
    //! The headline is the whole test surface here.
    //!
    //! A reader who stops after one line must still have the right answer, so
    //! these assert what that line says — and, more often, what it must never
    //! say. "No problems found" on a report where a rule could not be decided
    //! is the failure this module exists to prevent.

    use super::*;
    use openinvar_core::coverage::Coverage;
    use openinvar_core::rule_eval::{RuleOutcome, Uncertainty};
    use openinvar_core::rules::Severity;
    use std::collections::BTreeSet;

    fn outcome(name: &str, verdict: Verdict, severity: Severity) -> RuleOutcome {
        RuleOutcome {
            rule: name.to_string(),
            kind: "forbid-dependency",
            severity,
            verdict,
            violations: if verdict == Verdict::Violated {
                vec![openinvar_core::rule_eval::Violation {
                    file: "src/a.ts".to_string(),
                    line: 3,
                    symbol: "a".to_string(),
                    detail: "a -> b".to_string(),
                }]
            } else {
                vec![]
            },
            uncertainty: if verdict == Verdict::Inconclusive {
                Some(Uncertainty {
                    unresolved_edges: 4,
                    files: BTreeSet::new(),
                })
            } else {
                None
            },
            skipped_because: if verdict == Verdict::Skipped {
                Some("needs two graphs".to_string())
            } else {
                None
            },
        }
    }

    fn evaluation(outcomes: Vec<RuleOutcome>) -> Evaluation {
        Evaluation { outcomes }
    }

    fn empty_findings() -> DiffFindings {
        DiffFindings {
            findings: vec![],
            changed_file_coverage: Coverage::default(),
            blast_radius: BTreeSet::new(),
        }
    }

    #[test]
    fn a_blocking_violation_leads_the_report() {
        let evaluation = evaluation(vec![outcome("layering", Verdict::Violated, Severity::Error)]);
        let out = report("a", "b", None, None, Some(&evaluation));
        let headline = out.lines().nth(3).expect("headline line");

        assert!(headline.contains("1 rule(s) violated"), "{out}");
        assert!(headline.contains("blocked"), "{out}");
    }

    #[test]
    fn a_warn_violation_says_it_is_real_but_not_blocking() {
        let evaluation = evaluation(vec![outcome("advisory", Verdict::Violated, Severity::Warn)]);
        let out = report("a", "b", None, None, Some(&evaluation));

        assert!(out.contains("warn severity"), "{out}");
        assert!(out.contains("Not blocking"), "{out}");
        assert!(
            !out.contains("No problems found"),
            "a breach was found; saying otherwise is false:\n{out}"
        );
    }

    #[test]
    fn an_undecided_rule_prevents_the_report_reading_as_clean() {
        let evaluation = evaluation(vec![
            outcome("decided", Verdict::Satisfied, Severity::Error),
            outcome("undecided", Verdict::Inconclusive, Severity::Error),
        ]);
        let out = report("a", "b", None, None, Some(&evaluation));

        assert!(out.contains("could not be decided"), "{out}");
        assert!(out.contains("incomplete check"), "{out}");
        assert!(
            !out.contains("No problems found"),
            "the comment is what gets read; it must not claim a pass:\n{out}"
        );
    }

    #[test]
    fn a_skipped_rule_also_prevents_a_clean_reading() {
        let evaluation = evaluation(vec![outcome("needs-delta", Verdict::Skipped, Severity::Error)]);
        let out = report("a", "b", None, None, Some(&evaluation));

        assert!(!out.contains("No problems found"), "{out}");
    }

    #[test]
    fn a_fully_examined_clean_run_says_so_plainly() {
        let evaluation = evaluation(vec![outcome("layering", Verdict::Satisfied, Severity::Error)]);
        let out = report("a", "b", None, None, Some(&evaluation));

        assert!(out.contains("No problems found"), "{out}");
        assert!(out.contains("examined in full"), "{out}");
    }

    #[test]
    fn every_rule_appears_in_the_table_whatever_its_verdict() {
        // A reader has to be able to confirm the rule they care about ran,
        // which a summary count cannot tell them.
        let evaluation = evaluation(vec![
            outcome("one", Verdict::Satisfied, Severity::Error),
            outcome("two", Verdict::Violated, Severity::Error),
            outcome("three", Verdict::Inconclusive, Severity::Error),
            outcome("four", Verdict::Skipped, Severity::Error),
        ]);
        let out = report("a", "b", None, None, Some(&evaluation));

        for name in ["one", "two", "three", "four"] {
            assert!(out.contains(name), "rule '{name}' missing from:\n{out}");
        }
    }

    #[test]
    fn the_marker_is_first_so_a_later_run_finds_and_updates_this_comment() {
        let out = report("a", "b", None, None, None);
        assert!(
            out.starts_with(COMMENT_MARKER),
            "nine identical bot comments is a thread nobody reads:\n{out}"
        );
    }

    #[test]
    fn a_pipe_in_a_rule_name_cannot_break_the_table() {
        let mut broken = outcome("a|b", Verdict::Violated, Severity::Error);
        broken.rule = "a|b".to_string();
        let out = report("a", "b", None, None, Some(&evaluation(vec![broken])));

        assert!(out.contains(r"a\|b"), "{out}");
    }

    #[test]
    fn a_repository_with_no_rules_is_not_reported_as_having_passed_them() {
        // Caught by running the action against OpenInvar's own pull request:
        // the comment read "No problems found" on a repository that has no
        // rules file, which is exactly the unearned pass `check` refuses to
        // report. A reader cannot tell that from silence.
        let delta = GraphDelta {
            added_symbols: vec![],
            removed_symbols: vec![],
            continuities: vec![],
            signature_changes: vec![],
            added_edges: vec![],
            removed_edges: vec![],
        };
        let found = empty_findings();
        let out = report("a", "b", Some(&delta), Some(&found), None);

        assert!(
            out.contains("No `.openinvar/rules.toml`"),
            "the comment must say no rule was enforced:\n{out}"
        );
        assert!(
            out.contains("not a passing rule check"),
            "silence about rules reads as rules that passed:\n{out}"
        );
    }

    #[test]
    fn the_headline_without_rules_claims_only_what_the_diff_supports() {
        let evaluation: Option<&Evaluation> = None;
        let mut delta = GraphDelta::default();
        delta.added_symbols.push(openinvar_core::ir::Symbol {
            id: "a::b::function".to_string(),
            name: "b".to_string(),
            kind: openinvar_core::ir::SymbolKind::Function,
            language: openinvar_core::ir::Language::TypeScript,
            file: "a.ts".to_string(),
            line_start: 1,
            line_end: 1,
            signature: None,
        });
        let found = empty_findings();
        let out = report("a", "b", Some(&delta), Some(&found), evaluation);

        assert!(out.contains("No broken references"), "{out}");
        assert!(
            !out.contains("No problems found"),
            "with no rules evaluated this claims more than the diff supports:\n{out}"
        );
    }

    #[test]
    fn an_empty_delta_says_nothing_changed_rather_than_nothing_is_wrong() {
        let delta = GraphDelta::default();
        let found = empty_findings();
        let out = report("a", "b", Some(&delta), Some(&found), None);

        assert!(out.contains("No graph change"), "{out}");
    }
}
