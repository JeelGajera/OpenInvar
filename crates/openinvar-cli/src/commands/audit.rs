//! `openinvar audit` — detect reward-hacking in a change.
//!
//! The command behind the framework. Everything here follows from what an
//! audit finding is: an accusation that a change was made to pass a check
//! rather than to work, levelled at a person or an agent who may have done
//! nothing wrong.
//!
//! So the output has to earn belief rather than assert it. Every finding
//! carries the evidence behind it and the id someone would write down to
//! suppress it. Suppressed findings are shown, not hidden. Detectors that
//! were never run are named, because an absent check is otherwise
//! indistinguishable from one that found nothing — which is exactly the shape
//! of the failure this command exists to catch.

use std::collections::BTreeSet;

use openinvar_core::audit::{self, detectors, AuditContext, AuditReport};
use openinvar_core::delta;
use openinvar_core::graph::InvarGraph;
use openinvar_core::rules::Severity;
use openinvar_store::RocksGraphStore;

use crate::output;

/// Version of the JSON contract, read by hooks and CI.
const SCHEMA_VERSION: u32 = 1;

/// Nothing found at or above the requested severity.
pub const EXIT_CLEAN: i32 = 0;
/// Something was found.
pub const EXIT_FINDINGS: i32 = 1;
/// The audit could not run.
pub const EXIT_UNABLE: i32 = 2;

pub fn run(
    path: &str,
    base: &str,
    head: &str,
    severity: &str,
    json: bool,
) -> Result<i32, Box<dyn std::error::Error>> {
    let floor = match severity {
        "warn" => Severity::Warn,
        "error" => Severity::Error,
        other => {
            return Err(format!("unknown severity '{other}'. Expected 'warn' or 'error'").into())
        }
    };

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
    let suppressions = super::load_suppressions(&root)?;

    // Only files a Tier 1 adapter resolved. Computed from both graphs: a file
    // deleted by the change is still where a finding about its removal points.
    let (tier_one, out_of_scope) = tier_one_files(&before, &after);

    let context = AuditContext {
        before: &before,
        after: &after,
        delta: &computed,
        tier_one_files: &tier_one,
    };

    let mut report = audit::run(&detectors::all(), &context, &suppressions);
    report.out_of_scope_files = out_of_scope;

    let at_or_above: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.severity >= floor)
        .collect();
    let exit = if at_or_above.is_empty() {
        EXIT_CLEAN
    } else {
        EXIT_FINDINGS
    };

    if json {
        println!("{}", to_json(base, head, &report, floor, exit));
    } else {
        report_human(base, head, &report, floor);
    }

    Ok(exit)
}

/// Split every file the change touched into Tier 1 and everything else.
///
/// Tier is a property of the language adapter, so this is the CLI's job rather
/// than the core's — and it is what keeps an accusation off a file that was
/// analysed within-file only.
fn tier_one_files(before: &InvarGraph, after: &InvarGraph) -> (BTreeSet<String>, usize) {
    let mut tier_one = BTreeSet::new();
    let mut out_of_scope = 0usize;

    let mut seen = BTreeSet::new();
    for graph in [before, after] {
        for entry in graph.file_index.iter() {
            if !seen.insert(entry.key().clone()) {
                continue;
            }
            match openinvar_lang::for_path(entry.key()) {
                Some(spec) if spec.tier() == openinvar_lang::Tier::Resolved => {
                    tier_one.insert(entry.key().clone());
                }
                _ => out_of_scope += 1,
            }
        }
    }
    (tier_one, out_of_scope)
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

fn severity_tag(severity: Severity) -> String {
    match severity {
        Severity::Error => output::red("[error]"),
        Severity::Warn => output::yellow("[warn]"),
    }
}

fn report_human(base: &str, head: &str, report: &AuditReport, floor: Severity) {
    output::banner("audit");
    output::info(&format!("{base} → {head}"));
    output::blank();

    output::section("Result");
    output::stat_highlight("Errors", &report.count_of(Severity::Error).to_string());
    output::stat_highlight("Warnings", &report.count_of(Severity::Warn).to_string());
    output::stat_highlight("Suppressed", &report.suppressed.len().to_string());

    let shown: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.severity >= floor)
        .collect();

    if !shown.is_empty() {
        output::section("Findings");
        for finding in &shown {
            output::stat(
                &format!("  {}", finding.summary),
                &format!(
                    "{} {} ({})",
                    finding.detector,
                    severity_tag(finding.severity),
                    finding.confidence.as_str()
                ),
            );
            output::dim_line(&format!("      {}:{}", finding.file, finding.line));
            // The evidence, always. The reader is being asked to accept an
            // accusation, and "suspicious change" is not checkable.
            for evidence in &finding.evidence {
                output::dim_line(&format!(
                    "        {}:{} — {}",
                    evidence.file, evidence.line, evidence.detail
                ));
            }
            // The id someone would write down to silence it. Printing it here
            // is the difference between suppression being a documented act and
            // a thing people do by deleting the check.
            output::dim_line(&format!("      suppress with: {}", finding.id));
        }
    }

    if !report.suppressed.is_empty() {
        // Shown, never hidden. A gate that silently drops what it was told to
        // ignore is the thing this command exists to catch.
        output::section("Suppressed");
        for entry in &report.suppressed {
            output::stat(
                &format!("  {}", entry.finding.summary),
                if entry.reason.is_empty() {
                    "no reason given"
                } else {
                    &entry.reason
                },
            );
        }
    }

    if !report.stale_suppressions.is_empty() {
        output::section("Stale suppressions");
        for id in &report.stale_suppressions {
            output::stat(&format!("  {id}"), "matched nothing this run");
        }
        output::dim_line("  Remove these: they hide nothing and suggest they do.");
    }

    // Always printed. An absent detector is indistinguishable from one that
    // found nothing, and that ambiguity is the failure mode here.
    output::section("Coverage");
    output::stat("Detectors run", &report.detectors_run.join(", "));
    for held in detectors::held_back() {
        output::stat(&format!("  {} — not run", held.name), "");
        output::dim_line(&format!("      {}", held.reason));
    }
    if report.out_of_scope_files > 0 {
        output::stat(
            "Files out of scope",
            &format!(
                "{} (no Tier 1 adapter resolved them)",
                report.out_of_scope_files
            ),
        );
    }

    output::blank();
    if shown.is_empty() {
        if report.findings.is_empty() {
            output::success("No findings from the detectors that ran.");
        } else {
            output::success(&format!(
                "Nothing at {} severity or above.",
                floor.as_str()
            ));
        }
        output::dim_line("  This is not a clean bill of health: only the detectors listed");
        output::dim_line("  above ran, and only on files a Tier 1 adapter resolved.");
    } else {
        output::error(&format!("{} finding(s).", shown.len()));
    }
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

fn finding_json(finding: &openinvar_core::audit::Finding) -> String {
    let evidence: Vec<String> = finding
        .evidence
        .iter()
        .map(|e| {
            format!(
                r#"{{"file":"{}","line":{},"detail":"{}"}}"#,
                escape(&e.file),
                e.line,
                escape(&e.detail)
            )
        })
        .collect();
    format!(
        r#"{{"id":"{}","detector":"{}","severity":"{}","confidence":"{}","summary":"{}","file":"{}","line":{},"evidence":[{}]}}"#,
        escape(finding.id.as_str()),
        finding.detector,
        finding.severity.as_str(),
        finding.confidence.as_str(),
        escape(&finding.summary),
        escape(&finding.file),
        finding.line,
        evidence.join(",")
    )
}

fn to_json(
    base: &str,
    head: &str,
    report: &AuditReport,
    floor: Severity,
    exit: i32,
) -> String {
    let findings: Vec<String> = report
        .findings
        .iter()
        .filter(|f| f.severity >= floor)
        .map(finding_json)
        .collect();
    let suppressed: Vec<String> = report
        .suppressed
        .iter()
        .map(|s| {
            format!(
                r#"{{"finding":{},"reason":"{}"}}"#,
                finding_json(&s.finding),
                escape(&s.reason)
            )
        })
        .collect();
    let stale: Vec<String> = report
        .stale_suppressions
        .iter()
        .map(|id| format!(r#""{}""#, escape(id)))
        .collect();
    let ran: Vec<String> = report
        .detectors_run
        .iter()
        .map(|d| format!(r#""{d}""#))
        .collect();
    let held: Vec<String> = detectors::held_back()
        .iter()
        .map(|h| {
            format!(
                r#"{{"detector":"{}","reason":"{}"}}"#,
                h.name,
                escape(h.reason)
            )
        })
        .collect();

    format!(
        r#"{{"schema_version":{},"base":"{}","head":"{}","severity_floor":"{}","findings":[{}],"suppressed":[{}],"stale_suppressions":[{}],"detectors_run":[{}],"detectors_not_run":[{}],"out_of_scope_files":{},"exit_code":{}}}"#,
        SCHEMA_VERSION,
        escape(base),
        escape(head),
        floor.as_str(),
        findings.join(","),
        suppressed.join(","),
        stale.join(","),
        ran.join(","),
        held.join(","),
        report.out_of_scope_files,
        exit
    )
}
