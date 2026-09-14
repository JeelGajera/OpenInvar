//! Evaluating [`crate::rules`] against a graph.
//!
//! The schema decides whether a rules file is well formed. This decides what
//! the rules say about a repository.
//!
//! # A rule has three outcomes, not two
//!
//! The obvious design gives a rule a boolean: it passed, or it did not. That
//! design is wrong here, and wrongly reassuring.
//!
//! A rule like "nothing under `core/` may import `cli/`" is a claim about
//! *absence*. To report it satisfied is to claim every edge out of `core/` was
//! examined and none pointed at `cli/`. OpenInvar cannot always make that claim:
//! a [structural][crate::ir::Resolution::Structural] edge records that a call
//! to some `foo` happened, not which `foo` it reached. An unexamined edge
//! could be the violation. Reporting "satisfied" on that evidence is reporting
//! a pass the analysis has not earned — the one thing a deterministic gate
//! must never do.
//!
//! So a rule is [`Verdict::Satisfied`] only when every edge in its scope was
//! resolved. Where weaker edges could hide a violation the verdict is
//! [`Verdict::Inconclusive`], carrying the count of edges that prevented an
//! answer. Inconclusive is not a failure — it does not fail a gate — but it is
//! never silent either. It is the honest third answer, and the reason a Tier 2
//! language fails open rather than passing quietly.
//!
//! Violations, by contrast, are only ever reported on resolved evidence, so a
//! reported violation is a fact rather than a suspicion.

use std::collections::{BTreeMap, BTreeSet};

use crate::delta::GraphDelta;
use crate::graph::InvarGraph;
use crate::ir::{RelationshipKind, Symbol, SymbolKind};
use crate::rules::{Rule, RuleKind, Rules, Severity};
use crate::symbol_id::{is_external_package, parse_external_package_id};

/// What a rule concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    /// Examined in full, and nothing violates it.
    Satisfied,
    /// Violated, on resolved evidence.
    Violated,
    /// Could not be decided: evidence in scope was too weak to rule a
    /// violation out. Never reported as a pass.
    Inconclusive,
    /// Not attempted, because the inputs for it were not supplied.
    Skipped,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Satisfied => "satisfied",
            Self::Violated => "violated",
            Self::Inconclusive => "inconclusive",
            Self::Skipped => "skipped",
        }
    }
}

/// One concrete breach of a rule, with the evidence for it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Violation {
    /// Where the offending code is.
    pub file: String,
    pub line: u32,
    /// The symbol the breach is attributed to.
    pub symbol: String,
    /// What was found, in the terms the rule was written in.
    pub detail: String,
}

/// Why a rule could not be decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uncertainty {
    /// How many in-scope edges were too weakly resolved to judge.
    pub unresolved_edges: usize,
    /// The files those edges live in, deduplicated.
    pub files: BTreeSet<String>,
}

/// What one rule concluded, and on what evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleOutcome {
    pub rule: String,
    pub kind: &'static str,
    pub severity: Severity,
    pub verdict: Verdict,
    /// Non-empty exactly when `verdict` is [`Verdict::Violated`].
    pub violations: Vec<Violation>,
    /// Present exactly when `verdict` is [`Verdict::Inconclusive`].
    pub uncertainty: Option<Uncertainty>,
    /// Why the rule was skipped. Present exactly when `verdict` is
    /// [`Verdict::Skipped`].
    pub skipped_because: Option<String>,
}

/// The result of evaluating a whole rules file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Evaluation {
    /// One per rule, in the order the rules were written.
    pub outcomes: Vec<RuleOutcome>,
}

impl Evaluation {
    /// Outcomes that failed on resolved evidence at `error` severity.
    ///
    /// This, and only this, is what a gate fails on. Inconclusive and skipped
    /// rules are reported and do not fail — the fail-open half of the contract.
    pub fn blocking(&self) -> impl Iterator<Item = &RuleOutcome> {
        self.outcomes
            .iter()
            .filter(|o| o.verdict == Verdict::Violated && o.severity == Severity::Error)
    }

    /// Whether a gate reading this evaluation should fail.
    pub fn should_fail(&self) -> bool {
        self.blocking().next().is_some()
    }

    /// Outcomes that could not be decided. Always worth surfacing: each one is
    /// a question the analysis could not answer, not a rule that passed.
    pub fn inconclusive(&self) -> impl Iterator<Item = &RuleOutcome> {
        self.outcomes
            .iter()
            .filter(|o| o.verdict == Verdict::Inconclusive)
    }

    pub fn count_of(&self, verdict: Verdict) -> usize {
        self.outcomes.iter().filter(|o| o.verdict == verdict).count()
    }
}

// ── scope matching ───────────────────────────────────────────

/// The path a glob is matched against for a given symbol id.
///
/// Ordinary symbols match on the file they are defined in. An external package
/// node has no file, so it matches on its package name — which is what a rule
/// like `to = "openssl"` is reaching for.
fn scope_path<'g>(graph: &'g InvarGraph, id: &'g str) -> Option<String> {
    if let Some(symbol) = graph.symbols.get(id) {
        return Some(symbol.file.clone());
    }
    parse_external_package_id(id).map(|package| package.to_string())
}

fn matches(pattern: &glob::Pattern, path: &str) -> bool {
    pattern.matches(path)
}

/// Compile a glob that [`crate::rules`] already accepted.
///
/// Parsing rejects a pattern that does not compile, so reaching this with a bad
/// one is a bug rather than a user error. A pattern that matches nothing is the
/// safe reading: it cannot manufacture a violation.
fn compile(pattern: &str) -> glob::Pattern {
    glob::Pattern::new(pattern).unwrap_or_else(|_| {
        debug_assert!(false, "rules::parse accepted a glob that does not compile");
        glob::Pattern::new("[").unwrap_or_else(|_| glob::Pattern::new("").expect("empty glob"))
    })
}

/// Whether an edge kind counts as a dependency.
///
/// `forbid-dependency` is about the edges that make one file need another to
/// exist: imports, and re-exports that pass a symbol through. `forbid-reference`
/// is the superset — every way one symbol can mention another. Keeping both
/// means "you may call into this, but not import it" is expressible, which the
/// single combined rule could not say.
fn is_dependency(kind: &RelationshipKind) -> bool {
    matches!(kind, RelationshipKind::Imports | RelationshipKind::ReExports)
}

/// One edge, flattened out of the petgraph and into a sortable shape.
///
/// Collected and sorted rather than walked in place: `node_index` is a
/// `DashMap` and the graph's own edge order follows insertion, which follows
/// whichever order the parallel scan finished files in. Sorting here is what
/// makes two runs over the same repository produce the same violation list.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Edge {
    from: String,
    to: String,
    file: String,
    line: u32,
    kind_name: &'static str,
    /// Flattened here rather than kept as a [`RelationshipKind`] so the whole
    /// row is orderable; the kind itself carries no ordering.
    is_dependency: bool,
    gate_safe: bool,
}

fn edges(graph: &InvarGraph) -> Vec<Edge> {
    use petgraph::visit::EdgeRef as _;

    let mut out: Vec<Edge> = graph
        .graph
        .edge_references()
        .filter_map(|edge| {
            let from = graph.graph.node_weight(edge.source())?.clone();
            let to = graph.graph.node_weight(edge.target())?.clone();
            let meta = edge.weight();
            Some(Edge {
                from,
                to,
                file: meta.file.clone(),
                line: meta.line,
                kind_name: crate::query::kind_name(&meta.kind),
                is_dependency: is_dependency(&meta.kind),
                gate_safe: meta.resolution.is_gate_safe(),
            })
        })
        .collect();
    out.sort();
    out
}

// ── the evaluators ───────────────────────────────────────────

/// Shared body of `forbid-dependency` and `forbid-reference`.
///
/// Both ask the same question — does an edge run from one scope to another —
/// and differ only in which edge kinds count.
fn forbid_edges(
    graph: &InvarGraph,
    all_edges: &[Edge],
    from: &str,
    to: &str,
    dependency_only: bool,
) -> (Vec<Violation>, Uncertainty) {
    let from_pattern = compile(from);
    let to_pattern = compile(to);

    let mut violations = Vec::new();
    let mut unresolved_edges = 0usize;
    let mut unresolved_files = BTreeSet::new();

    for edge in all_edges {
        if dependency_only && !edge.is_dependency {
            continue;
        }
        let Some(source_path) = scope_path(graph, &edge.from) else {
            continue;
        };
        if !matches(&from_pattern, &source_path) {
            continue;
        }

        // In scope. A weakly resolved edge out of the source scope could point
        // anywhere, including at the forbidden target, so it is counted as
        // uncertainty whether or not its recorded target matches.
        if !edge.gate_safe {
            unresolved_edges += 1;
            unresolved_files.insert(edge.file.clone());
            continue;
        }

        let Some(target_path) = scope_path(graph, &edge.to) else {
            continue;
        };
        if matches(&to_pattern, &target_path) {
            violations.push(Violation {
                file: edge.file.clone(),
                line: edge.line,
                symbol: edge.from.clone(),
                detail: format!(
                    "{} -> {} ({}), forbidden from '{}' to '{}'",
                    source_path, target_path, edge.kind_name, from, to
                ),
            });
        }
    }

    violations.sort();
    (
        violations,
        Uncertainty {
            unresolved_edges,
            files: unresolved_files,
        },
    )
}

/// `layers`: an ordered stack, outermost first.
///
/// Evaluated natively rather than desugared into one `forbid_edges` call per
/// upward pair, for three reasons.
///
/// One declared rule stays one reported row. Desugaring a five-layer stack
/// produces ten synthetic rules, so three broken pairs would read as three
/// failures of a rule the author wrote once.
///
/// A violation can name the layers. `crates/core` (layer 3) importing
/// `crates/cli` (layer 0) is a sentence; glob A touched glob B is not, and the
/// ordinal is the part that tells a reader which way the arrow was supposed to
/// point.
///
/// **And uncertainty is asymmetric.** The top layer may depend on everything,
/// so an unresolved edge leaving it cannot be an upward violation — there is
/// nothing above it to violate. A desugared evaluator counts that edge and
/// reports the rule inconclusive on evidence that could never have mattered.
/// Here, an edge is only uncertain when it leaves a layer that *has* layers
/// above it.
fn layers(
    graph: &InvarGraph,
    all_edges: &[Edge],
    layers: &[String],
) -> (Vec<Violation>, Uncertainty) {
    let patterns: Vec<glob::Pattern> = layers.iter().map(|l| compile(l)).collect();

    // Which layer a path belongs to, by index. First match wins, so an earlier
    // (higher) layer claims a path that two globs would both accept.
    let layer_of = |path: &str| -> Option<usize> {
        patterns.iter().position(|pattern| matches(pattern, path))
    };

    let mut violations = Vec::new();
    let mut unresolved_edges = 0usize;
    let mut unresolved_files = BTreeSet::new();

    for edge in all_edges {
        if !edge.is_dependency {
            continue;
        }
        let Some(source_path) = scope_path(graph, &edge.from) else {
            continue;
        };
        let Some(source_layer) = layer_of(&source_path) else {
            continue;
        };

        // The asymmetry. A layer with nothing above it cannot depend upward,
        // so its weak edges carry no uncertainty for this rule.
        if source_layer == 0 {
            continue;
        }

        if !edge.gate_safe {
            unresolved_edges += 1;
            unresolved_files.insert(edge.file.clone());
            continue;
        }

        let Some(target_path) = scope_path(graph, &edge.to) else {
            continue;
        };
        let Some(target_layer) = layer_of(&target_path) else {
            continue;
        };

        // Downward and same-layer are both allowed: a layer is a boundary
        // against reaching up, not against internal cohesion.
        if target_layer < source_layer {
            violations.push(Violation {
                file: edge.file.clone(),
                line: edge.line,
                symbol: edge.from.clone(),
                detail: format!(
                    "{source_path} (layer {source_layer}: '{}') depends upward on \
                     {target_path} (layer {target_layer}: '{}') via {}",
                    layers[source_layer], layers[target_layer], edge.kind_name
                ),
            });
        }
    }

    violations.sort();
    (
        violations,
        Uncertainty {
            unresolved_edges,
            files: unresolved_files,
        },
    )
}

/// `independence`: siblings that must not reference each other, either way.
///
/// Unlike `layers` there is no permitted direction, so every module's weak
/// edges are genuinely uncertain — an unresolved edge leaving any of them
/// could land in any of the others.
///
/// Uncertainty is counted once per edge rather than once per module pair. The
/// same file appearing k times for k modules would say the evidence is k times
/// weaker than it is.
fn independence(
    graph: &InvarGraph,
    all_edges: &[Edge],
    modules: &[String],
) -> (Vec<Violation>, Uncertainty) {
    let patterns: Vec<glob::Pattern> = modules.iter().map(|m| compile(m)).collect();
    let module_of = |path: &str| -> Option<usize> {
        patterns.iter().position(|pattern| matches(pattern, path))
    };

    let mut violations = Vec::new();
    let mut unresolved_edges = 0usize;
    let mut unresolved_files = BTreeSet::new();

    for edge in all_edges {
        let Some(source_path) = scope_path(graph, &edge.from) else {
            continue;
        };
        let Some(source_module) = module_of(&source_path) else {
            continue;
        };

        if !edge.gate_safe {
            unresolved_edges += 1;
            unresolved_files.insert(edge.file.clone());
            continue;
        }

        let Some(target_path) = scope_path(graph, &edge.to) else {
            continue;
        };
        let Some(target_module) = module_of(&target_path) else {
            continue;
        };

        if source_module != target_module {
            violations.push(Violation {
                file: edge.file.clone(),
                line: edge.line,
                symbol: edge.from.clone(),
                detail: format!(
                    "{source_path} ('{}') references {target_path} ('{}') via {}; \
                     these are declared independent",
                    modules[source_module], modules[target_module], edge.kind_name
                ),
            });
        }
    }

    violations.sort();
    (
        violations,
        Uncertainty {
            unresolved_edges,
            files: unresolved_files,
        },
    )
}

/// `max-fan-in`: how many things point at one symbol.
///
/// Fan-in is counted from resolved edges only, so a reported breach is real.
/// A symbol whose resolved fan-in is within the threshold but whose weaker
/// edges could carry it over is uncertainty rather than a pass: the count that
/// matters is one nobody can see.
///
/// External package nodes are not counted. Fan-in on a third-party package
/// measures how much a repository uses a library, not how coupled its own code
/// has become — every real codebase points at `serde` or `express` hundreds of
/// times, and a rule that flags that is one whose threshold gets raised until
/// it means nothing.
fn max_fan_in(graph: &InvarGraph, all_edges: &[Edge], threshold: usize) -> (Vec<Violation>, Uncertainty) {
    let mut resolved: BTreeMap<&str, usize> = BTreeMap::new();
    let mut weak: BTreeMap<&str, usize> = BTreeMap::new();

    for edge in all_edges {
        if is_external_package(&edge.to) {
            continue;
        }
        let counter = if edge.gate_safe {
            &mut resolved
        } else {
            &mut weak
        };
        *counter.entry(edge.to.as_str()).or_default() += 1;
    }

    let mut violations = Vec::new();
    let mut unresolved_edges = 0usize;
    let mut unresolved_files = BTreeSet::new();

    let candidates: BTreeSet<&str> = resolved.keys().copied().chain(weak.keys().copied()).collect();
    for id in candidates {
        let strong = resolved.get(id).copied().unwrap_or(0);
        let uncertain = weak.get(id).copied().unwrap_or(0);

        if strong > threshold {
            // Falling back to the id rather than an empty path: a violation
            // that renders as ":0" tells the reader nothing about what broke.
            let (file, line) = graph
                .symbols
                .get(id)
                .map(|s| (s.file.clone(), s.line_start))
                .unwrap_or_else(|| (id.to_string(), 0));
            violations.push(Violation {
                file,
                line,
                symbol: id.to_string(),
                detail: format!("{strong} inbound references, limit is {threshold}"),
            });
        } else if strong + uncertain > threshold {
            // Could be over the line; the evidence cannot say.
            unresolved_edges += uncertain;
            if let Some(symbol) = graph.symbols.get(id) {
                unresolved_files.insert(symbol.file.clone());
            }
        }
    }

    violations.sort();
    (
        violations,
        Uncertainty {
            unresolved_edges,
            files: unresolved_files,
        },
    )
}

/// Whether a symbol kind is a field for the purpose of `no-field-removal`.
///
/// Fields only. A removed method is an API change that `findings` already
/// reports as removed surface; conflating the two would make the rule fire on
/// changes it does not name.
fn is_field(kind: &SymbolKind) -> bool {
    matches!(kind, SymbolKind::Property | SymbolKind::EnumVariant)
}

/// The symbol a removed field belonged to, if any.
///
/// Symbol ids encode no parent and member naming differs per language, so
/// ownership has to be recovered from position. Two adapters disagree about
/// what a container's line range means, and both have to work:
///
/// * Some record a real span, `class Foo {` through its closing brace. There
///   containment is exact, and the innermost enclosing container wins, so a
///   field of a nested class is not attributed to the outer one.
/// * Others record only the declaration line, leaving `line_end` equal to
///   `line_start` — TypeScript does this today, which is how a rule naming a
///   class with four fields could report nothing at all. There the owner is
///   the nearest container declared above the field.
///
/// The fallback is a heuristic and can misattribute a field declared after a
/// container's body has closed but before the next one opens. It is the
/// honest trade: the alternative is a rule that silently never fires.
fn owner_of<'a>(containers: &'a [Symbol], field: &Symbol) -> Option<&'a Symbol> {
    // An exact range beats a guess, and the innermost of those beats an outer.
    let enclosing = containers
        .iter()
        .filter(|c| {
            c.line_end > c.line_start
                && c.line_start <= field.line_start
                && field.line_end <= c.line_end
        })
        .max_by_key(|c| (c.line_start, c.id.as_str()));
    if enclosing.is_some() {
        return enclosing;
    }

    containers
        .iter()
        .filter(|c| c.line_start <= field.line_start)
        .max_by_key(|c| (c.line_start, c.id.as_str()))
}

/// Whether a symbol can own a field.
///
/// The synthetic per-file module symbol is excluded deliberately: it sits at
/// line 1 of every file, so leaving it in would make it the fallback owner of
/// every field in the repository.
fn can_own_fields(symbol: &Symbol) -> bool {
    !is_field(&symbol.kind) && !matches!(symbol.kind, SymbolKind::Module)
}

/// `no-field-removal`: a named symbol may not lose fields.
fn no_field_removal(before: &InvarGraph, delta: &GraphDelta, symbol_glob: &str) -> Vec<Violation> {
    let pattern = compile(symbol_glob);

    // A symbol deleted outright has no fields left to lose, and reporting each
    // of its fields would bury the one finding that matters — that the symbol
    // went. `findings` already reports the removal itself.
    let removed_ids: BTreeSet<&str> = delta
        .removed_symbols
        .iter()
        .map(|s| s.id.as_str())
        .collect();

    // Candidate owners are grouped per file, sorted, and taken from the graph
    // the fields were removed from.
    let mut by_file: BTreeMap<String, Vec<Symbol>> = BTreeMap::new();
    for entry in before.symbols.iter() {
        let symbol = entry.value();
        if can_own_fields(symbol) && !removed_ids.contains(symbol.id.as_str()) {
            by_file
                .entry(symbol.file.clone())
                .or_default()
                .push(symbol.clone());
        }
    }
    for containers in by_file.values_mut() {
        containers.sort_by(|a, b| (a.line_start, &a.id).cmp(&(b.line_start, &b.id)));
    }

    let mut violations = Vec::new();
    for removed in &delta.removed_symbols {
        if !is_field(&removed.kind) {
            continue;
        }
        let Some(containers) = by_file.get(&removed.file) else {
            continue;
        };
        let Some(owner) = owner_of(containers, removed) else {
            continue;
        };
        if !matches(&pattern, &owner.name) {
            continue;
        }
        violations.push(Violation {
            file: removed.file.clone(),
            line: removed.line_start,
            symbol: owner.id.clone(),
            detail: format!("field '{}' removed from '{}'", removed.name, owner.name),
        });
    }

    violations.sort();
    violations
}

// ── entry point ──────────────────────────────────────────────

/// Evaluate every rule against a graph, and a delta where one is supplied.
///
/// `delta` is the change being judged, with `graph` as its "before". Rules that
/// need a delta are [`Verdict::Skipped`] when it is absent, never reported as
/// satisfied: a caller holding one graph has not shown that no field was
/// removed, only that it did not look.
pub fn evaluate(rules: &Rules, graph: &InvarGraph, delta: Option<&GraphDelta>) -> Evaluation {
    let all_edges = edges(graph);

    let outcomes = rules
        .rules
        .iter()
        .map(|rule| evaluate_one(rule, graph, &all_edges, delta))
        .collect();

    Evaluation { outcomes }
}

fn evaluate_one(
    rule: &Rule,
    graph: &InvarGraph,
    all_edges: &[Edge],
    delta: Option<&GraphDelta>,
) -> RuleOutcome {
    let mut outcome = RuleOutcome {
        rule: rule.name.clone(),
        kind: rule.kind.as_str(),
        severity: rule.severity,
        verdict: Verdict::Satisfied,
        violations: Vec::new(),
        uncertainty: None,
        skipped_because: None,
    };

    let (violations, uncertainty) = match &rule.kind {
        RuleKind::ForbidDependency { from, to } => {
            let (v, u) = forbid_edges(graph, all_edges, from, to, true);
            (v, Some(u))
        }
        RuleKind::ForbidReference { from, to } => {
            let (v, u) = forbid_edges(graph, all_edges, from, to, false);
            (v, Some(u))
        }
        RuleKind::Layers { layers: stack } => {
            let (v, u) = layers(graph, all_edges, stack);
            (v, Some(u))
        }
        RuleKind::Independence { modules } => {
            let (v, u) = independence(graph, all_edges, modules);
            (v, Some(u))
        }
        RuleKind::MaxFanIn { threshold } => {
            let (v, u) = max_fan_in(graph, all_edges, *threshold);
            (v, Some(u))
        }
        RuleKind::NoFieldRemoval { symbol } => match delta {
            Some(delta) => (no_field_removal(graph, delta, symbol), None),
            None => {
                outcome.verdict = Verdict::Skipped;
                outcome.skipped_because =
                    Some("needs two graphs to compare; none was supplied".to_string());
                return outcome;
            }
        },
    };

    // Violation beats uncertainty. Weak evidence elsewhere does not make a
    // breach found on resolved evidence any less of a breach.
    if !violations.is_empty() {
        outcome.verdict = Verdict::Violated;
        outcome.violations = violations;
    } else if let Some(uncertainty) = uncertainty {
        if uncertainty.unresolved_edges > 0 {
            outcome.verdict = Verdict::Inconclusive;
            outcome.uncertainty = Some(uncertainty);
        }
    }

    outcome
}

