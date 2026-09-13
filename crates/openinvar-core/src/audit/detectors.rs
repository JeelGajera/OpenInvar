//! The detectors.
//!
//! # Precision is the only thing that matters here
//!
//! A gate that cries wolf is switched off in week two, and a switched-off gate
//! catches nothing. So a detector ships only if it is near-silent on ordinary
//! work, and one that cannot reach that bar is held back rather than
//! weakened — a detector at "medium" that fires on every second pull request
//! costs more trust than it earns.
//!
//! Three ship here. Three are held back, and the reasons are recorded in
//! [`held_back`] rather than left as an absence somebody has to rediscover.

use std::collections::{BTreeMap, BTreeSet};

use petgraph::visit::EdgeRef as _;
use petgraph::Direction;

use crate::audit::{AuditContext, Confidence, Detector, Evidence, Finding, FindingId};
use crate::delta::GraphDelta;
use crate::graph::InvarGraph;
use crate::ir::{RelationshipKind, SymbolId};
use crate::rules::Severity;

/// Every detector this build carries, in a fixed order.
pub fn all() -> Vec<&'static dyn Detector> {
    vec![
        &TestTampering as &'static dyn Detector,
        &ContractErosion as &'static dyn Detector,
        &DeadOnArrival as &'static dyn Detector,
    ]
}

/// A detector that was specified but is not shipped, and why.
///
/// Recorded rather than omitted. An absent detector is indistinguishable from
/// one that found nothing, and someone reading the audit output needs to know
/// which checks were never made.
pub struct HeldBack {
    pub name: &'static str,
    pub reason: &'static str,
}

pub fn held_back() -> &'static [HeldBack] {
    &[
        HeldBack {
            name: "assertion-removal",
            reason: "assertion calls are not in the graph. Rust's assert macros are \
                     token trees that are never expanded, and framework calls like \
                     expect() resolve to nothing, so a net assertion count cannot be \
                     computed from the graph at all. Counting them would mean parsing \
                     source text outside the graph, which is a second source of truth.",
        },
        HeldBack {
            name: "scope-creep",
            reason: "there is no declared task scope to compare a blast radius against. \
                     OpenInvar is given a repository and a revision range, never an \
                     intended scope, so the detector has no denominator.",
        },
        HeldBack {
            name: "special-casing",
            reason: "identifying a new conditional branch whose only reachable caller is \
                     a test needs branch-level analysis. The graph models symbols and \
                     references between them, not control flow inside a function.",
        },
    ]
}

// ── shared helpers ───────────────────────────────────────────

/// Every symbol id the delta touched.
fn changed_ids(delta: &GraphDelta) -> BTreeSet<SymbolId> {
    let mut out = BTreeSet::new();
    for symbol in delta.added_symbols.iter().chain(delta.removed_symbols.iter()) {
        out.insert(symbol.id.clone());
    }
    for c in &delta.continuities {
        out.insert(c.before.id.clone());
        out.insert(c.after.id.clone());
    }
    for c in &delta.signature_changes {
        out.insert(c.before.id.clone());
        out.insert(c.after.id.clone());
    }
    for e in delta.added_edges.iter().chain(delta.removed_edges.iter()) {
        out.insert(e.from.clone());
    }
    out
}

/// Symbols a graph knows, for existence checks.
fn ids_in(graph: &InvarGraph) -> BTreeSet<SymbolId> {
    graph.symbols.iter().map(|e| e.key().clone()).collect()
}

/// `tests` edges in a graph, as test → covered.
fn coverage(graph: &InvarGraph) -> BTreeSet<(SymbolId, SymbolId)> {
    graph
        .graph
        .edge_references()
        .filter(|e| e.weight().kind == RelationshipKind::Tests)
        .filter_map(|e| {
            let from = graph.graph.node_weight(e.source())?.clone();
            let to = graph.graph.node_weight(e.target())?.clone();
            Some((from, to))
        })
        .collect()
}

fn file_of(graph: &InvarGraph, id: &str) -> String {
    graph
        .symbols
        .get(id)
        .map(|s| s.file.clone())
        .unwrap_or_default()
}

fn line_of(graph: &InvarGraph, id: &str) -> u32 {
    graph.symbols.get(id).map(|s| s.line_start).unwrap_or(0)
}

fn name_of(graph: &InvarGraph, id: &str) -> String {
    graph
        .symbols
        .get(id)
        .map(|s| s.name.clone())
        .unwrap_or_else(|| id.to_string())
}

// ── test tampering ───────────────────────────────────────────

/// Coverage disappeared from a symbol that changed in the same diff.
///
/// # Why not simply "a test changed alongside the code it covers"
///
/// That is what the brief asks for literally, and it would fire on almost
/// every honest pull request: changing a function and updating its test
/// together is the most ordinary thing a developer does. A detector with that
/// rule would be switched off in a week, which is worse than not shipping it.
///
/// The suspicious shape is narrower and genuinely rare: a test *stopped*
/// covering something in the same change that altered it. Either the test was
/// deleted, or its references to the symbol were removed while the symbol
/// stayed. Coverage vanishing exactly where behaviour changed is hard to do by
/// accident and is precisely what reward-hacking looks like from the graph.
pub struct TestTampering;

impl Detector for TestTampering {
    fn name(&self) -> &'static str {
        "test-tampering"
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn confidence(&self) -> Confidence {
        Confidence::High
    }
    fn describes(&self) -> &'static str {
        "a test stopped covering a symbol that changed in the same diff"
    }

    fn detect(&self, ctx: &AuditContext<'_>) -> Vec<Finding> {
        let changed = changed_ids(ctx.delta);
        let before = coverage(ctx.before);
        let after = coverage(ctx.after);
        let after_ids = ids_in(ctx.after);

        let mut findings = Vec::new();
        for (test, covered) in &before {
            if after.contains(&(test.clone(), covered.clone())) {
                continue;
            }
            // The covered symbol has to still exist and have changed. A symbol
            // deleted outright takes its coverage with it, which is a coherent
            // removal rather than a test being quietly defanged.
            if !after_ids.contains(covered) || !changed.contains(covered) {
                continue;
            }

            let file = file_of(ctx.before, test);
            let test_removed = !after_ids.contains(test);
            findings.push(Finding {
                id: FindingId::new(self.name(), &[test.as_str(), covered.as_str()]),
                detector: self.name(),
                severity: self.severity(),
                confidence: self.confidence(),
                // Named by file rather than by symbol. An adapter attributes
                // a reference to the nearest enclosing symbol, which for a
                // test body is often a local variable — dogfooding produced
                // "'p' stopped covering 'encode'", where `p` was a local. A
                // test is also the thing a reader runs, and you run a file.
                summary: format!(
                    "{} stopped covering '{}', which changed in the same diff",
                    file, name_of(ctx.before, covered)
                ),
                file: file.clone(),
                line: line_of(ctx.before, test),
                evidence: vec![
                    Evidence {
                        file,
                        line: line_of(ctx.before, test),
                        detail: if test_removed {
                            format!("'{}' was removed", name_of(ctx.before, test))
                        } else {
                            format!("'{}' no longer references it", name_of(ctx.before, test))
                        },
                    },
                    Evidence {
                        file: file_of(ctx.after, covered),
                        line: line_of(ctx.after, covered),
                        detail: format!(
                            "'{}' changed in this diff",
                            name_of(ctx.after, covered)
                        ),
                    },
                ],
            });
        }
        findings
    }
}

// ── contract erosion ─────────────────────────────────────────

/// A symbol was removed while something that still exists referred to it.
///
/// The callee was deleted rather than the caller fixed. Restricted to resolved
/// evidence and to referrers that survived the change: when the referrer went
/// too, the deletion is coherent rather than erosion.
///
/// Visibility is not modelled anywhere in the IR, so "public API" cannot be
/// distinguished from private. Every surviving resolved reference is treated as
/// a contract, which is the conservative reading — a private symbol with a live
/// caller is still a caller left broken.
pub struct ContractErosion;

impl Detector for ContractErosion {
    fn name(&self) -> &'static str {
        "contract-erosion"
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn confidence(&self) -> Confidence {
        Confidence::High
    }
    fn describes(&self) -> &'static str {
        "a symbol was removed while a surviving caller still referred to it"
    }

    fn detect(&self, ctx: &AuditContext<'_>) -> Vec<Finding> {
        let after_ids = ids_in(ctx.after);
        let mut findings = Vec::new();

        for removed in &ctx.delta.removed_symbols {
            let Some(node) = ctx.before.node_index.get(&removed.id).map(|v| *v) else {
                continue;
            };

            let mut referrers: BTreeSet<(String, String, u32)> = BTreeSet::new();
            for edge in ctx.before.graph.edges_directed(node, Direction::Incoming) {
                // Only evidence a gate may act on, and never a test: a test
                // losing its subject is the test-tampering question, not this
                // one, and counting it here would report both for one event.
                if !edge.weight().resolution.is_gate_safe()
                    || edge.weight().kind == RelationshipKind::Tests
                {
                    continue;
                }
                let Some(from) = ctx.before.graph.node_weight(edge.source()) else {
                    continue;
                };
                if !after_ids.contains(from) {
                    continue;
                }
                referrers.insert((
                    from.clone(),
                    edge.weight().file.clone(),
                    edge.weight().line,
                ));
            }

            if referrers.is_empty() {
                continue;
            }

            let mut subjects: Vec<&str> = vec![removed.id.as_str()];
            subjects.extend(referrers.iter().map(|(id, _, _)| id.as_str()));

            findings.push(Finding {
                id: FindingId::new(self.name(), &subjects),
                detector: self.name(),
                severity: self.severity(),
                confidence: self.confidence(),
                summary: format!(
                    "'{}' was removed, but {} surviving reference(s) still point at it",
                    removed.name,
                    referrers.len()
                ),
                file: removed.file.clone(),
                line: removed.line_start,
                evidence: referrers
                    .iter()
                    .map(|(id, file, line)| Evidence {
                        file: file.clone(),
                        line: *line,
                        detail: format!("'{}' still refers to it", name_of(ctx.before, id)),
                    })
                    .collect(),
            });
        }
        findings
    }
}

// ── dead on arrival ──────────────────────────────────────────

/// A new symbol nothing but a test refers to.
///
/// Code written to satisfy a test rather than to be used. The requirement that
/// a test *does* reference it is what keeps this precise: a genuinely new entry
/// point — a CLI command, an exported function a consumer outside the
/// repository calls — has no inbound edges at all and is not reported. Only a
/// symbol reached by tests and by nothing else is.
pub struct DeadOnArrival;

impl Detector for DeadOnArrival {
    fn name(&self) -> &'static str {
        "dead-on-arrival"
    }
    fn severity(&self) -> Severity {
        Severity::Warn
    }
    fn confidence(&self) -> Confidence {
        Confidence::High
    }
    fn describes(&self) -> &'static str {
        "a new symbol that only tests refer to"
    }

    fn detect(&self, ctx: &AuditContext<'_>) -> Vec<Finding> {
        let mut findings = Vec::new();

        for added in &ctx.delta.added_symbols {
            let Some(node) = ctx.after.node_index.get(&added.id).map(|v| *v) else {
                continue;
            };

            let mut from_tests: BTreeMap<String, (String, u32)> = BTreeMap::new();
            let mut from_anything_else = false;

            for edge in ctx.after.graph.edges_directed(node, Direction::Incoming) {
                let Some(from) = ctx.after.graph.node_weight(edge.source()) else {
                    continue;
                };
                if edge.weight().kind == RelationshipKind::Tests {
                    from_tests.insert(
                        from.clone(),
                        (edge.weight().file.clone(), edge.weight().line),
                    );
                    continue;
                }
                // A test's own call edge is not independent use: the `tests`
                // edge is derived from it, so counting it would make every
                // tested symbol look used.
                if from_tests.contains_key(from) || is_covered_by(ctx.after, from, &added.id) {
                    continue;
                }
                from_anything_else = true;
                break;
            }

            if from_anything_else || from_tests.is_empty() {
                continue;
            }

            let mut subjects: Vec<&str> = vec![added.id.as_str()];
            subjects.extend(from_tests.keys().map(|k| k.as_str()));

            findings.push(Finding {
                id: FindingId::new(self.name(), &subjects),
                detector: self.name(),
                severity: self.severity(),
                confidence: self.confidence(),
                summary: format!(
                    "'{}' is new and only tests refer to it",
                    added.name
                ),
                file: added.file.clone(),
                line: added.line_start,
                evidence: from_tests
                    .iter()
                    .map(|(id, (file, line))| Evidence {
                        file: file.clone(),
                        line: *line,
                        detail: format!("'{}' is the only kind of caller", name_of(ctx.after, id)),
                    })
                    .collect(),
            });
        }
        findings
    }
}

/// Whether `from` covers `target` with a `tests` edge.
fn is_covered_by(graph: &InvarGraph, from: &str, target: &str) -> bool {
    let Some(node) = graph.node_index.get(from).map(|v| *v) else {
        return false;
    };
    graph
        .graph
        .edges_directed(node, Direction::Outgoing)
        .any(|e| {
            e.weight().kind == RelationshipKind::Tests
                && graph.graph.node_weight(e.target()).is_some_and(|t| t == target)
        })
}
