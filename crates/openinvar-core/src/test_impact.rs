//! Which tests exercise a change.
//!
//! # Why this needs the same care as "safe to modify"
//!
//! Every other query here answers a question. This one licenses an *omission*:
//! a developer or an agent reads the answer and does not run the rest of the
//! suite. "These are the tests that cover your change" is therefore a claim
//! that the tests it leaves out cannot fail, and that claim is only as good as
//! the graph behind it.
//!
//! OpenInvar's graph is knowingly incomplete in two ways that matter here. A
//! structural region records no cross-file references at all, so a test living
//! there could exercise the changed code without any edge to show for it. And
//! test detection is by file convention, so a test OpenInvar did not recognise
//! as a test contributes nothing.
//!
//! So a [`TestSelection`] never says "run only these". It reports what it
//! found, and separately whether the selection is [complete enough to act
//! on][TestSelection::is_trustworthy]. A caller that ignores the second field
//! is making the omission claim on its own authority, not on OpenInvar's.

use std::collections::{BTreeMap, BTreeSet};

use petgraph::visit::EdgeRef as _;
use petgraph::Direction;

use crate::graph::InvarGraph;
use crate::ir::{RelationshipKind, Resolution, SymbolId};

/// One test, and what it reaches that the change touched.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CoveringTest {
    /// The test symbol's id.
    pub symbol: SymbolId,
    /// The file it lives in — what a test runner is actually given.
    pub file: String,
    /// Which changed symbols it reaches, sorted.
    pub covers: BTreeSet<SymbolId>,
    /// Whether every edge that put it here was resolved.
    ///
    /// A test selected on structural evidence may not exercise the change at
    /// all: the reference was matched by name inside one file.
    pub gate_safe: bool,
}

/// Why a selection cannot be trusted to be complete.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Gap {
    /// Files analysed within-file only. A test in one of these could exercise
    /// the change without recording an edge.
    StructuralRegions(usize),
    /// Changed symbols nothing recognised as a test reaches.
    ///
    /// Either genuinely untested, or tested by something OpenInvar did not
    /// recognise as a test. It cannot tell the two apart, and reporting the
    /// first when it means the second is how a gate lies.
    UncoveredSymbols(usize),
    /// A test was selected on evidence too weak to stand behind.
    StructuralEvidence(usize),
}

impl Gap {
    pub fn describe(&self) -> String {
        match self {
            Self::StructuralRegions(n) => format!(
                "{n} file(s) were analyzed within-file only; a test in one of them \
                 could exercise this change without recording an edge"
            ),
            Self::UncoveredSymbols(n) => format!(
                "{n} changed symbol(s) are reached by no recognized test — either \
                 untested, or tested by something not recognized as a test"
            ),
            Self::StructuralEvidence(n) => format!(
                "{n} test(s) were selected on structural evidence, which cannot \
                 see across files"
            ),
        }
    }
}

/// The tests that cover a change, and how much of the answer to believe.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TestSelection {
    /// Tests reaching at least one changed symbol, sorted.
    pub tests: Vec<CoveringTest>,
    /// Tests that were themselves changed.
    ///
    /// A modified test is not evidence that the code it covers still works —
    /// it is the thing whose behaviour changed. Separated so a caller can run
    /// them without counting them as coverage. This is what PR 21 reads.
    pub modified_tests: Vec<CoveringTest>,
    /// Changed symbols no recognized test reaches, sorted.
    pub uncovered: BTreeSet<SymbolId>,
    /// Every reason this selection may be incomplete.
    pub gaps: Vec<Gap>,
}

impl TestSelection {
    /// The files a runner would be given, deduplicated and sorted.
    pub fn files(&self) -> BTreeSet<&str> {
        self.tests.iter().map(|t| t.file.as_str()).collect()
    }

    /// Whether the selection is complete enough to run instead of the suite.
    ///
    /// False whenever anything could be missing. Deliberately strict: the cost
    /// of running the whole suite when this returns false is minutes, and the
    /// cost of trusting it when it should have been false is a regression that
    /// reached main with a green tick beside it.
    pub fn is_trustworthy(&self) -> bool {
        self.gaps.is_empty()
    }
}

/// Select the tests covering `changed`.
///
/// `depth` bounds how far a change propagates before a test counts as covering
/// it: a test that calls a function that calls the changed one is covering it
/// at depth 2. Zero means direct references only.
pub fn select(
    graph: &InvarGraph,
    changed: &BTreeSet<SymbolId>,
    depth: usize,
    min_resolution: Resolution,
) -> TestSelection {
    if changed.is_empty() {
        return TestSelection::default();
    }

    // Everything the change can reach backwards, with the changed symbol that
    // each reachable symbol traces to. A test covering any of these covers the
    // change, and the mapping is what lets the report say which.
    let reachable = reverse_reachable(graph, changed, depth, min_resolution);

    // A test is a symbol with at least one outgoing `tests` edge: the kind is
    // only ever minted for a symbol in a recognized test file.
    let mut by_test: BTreeMap<SymbolId, (BTreeSet<SymbolId>, bool)> = BTreeMap::new();

    for edge in graph.graph.edge_references() {
        if edge.weight().kind != RelationshipKind::Tests {
            continue;
        }
        let Some(from) = graph.graph.node_weight(edge.source()) else {
            continue;
        };
        let Some(to) = graph.graph.node_weight(edge.target()) else {
            continue;
        };
        // The test reaches `to`; `to` matters if the change reaches it.
        let Some(origins) = reachable.get(to.as_str()) else {
            continue;
        };
        let entry = by_test
            .entry(from.clone())
            .or_insert_with(|| (BTreeSet::new(), true));
        entry.0.extend(origins.iter().cloned());
        entry.1 &= edge.weight().resolution.is_gate_safe();
    }

    let mut tests = Vec::new();
    let mut modified_tests = Vec::new();
    let mut structural_evidence = 0usize;
    let mut covered: BTreeSet<SymbolId> = BTreeSet::new();

    for (symbol, (covers, gate_safe)) in by_test {
        let file = graph
            .symbols
            .get(&symbol)
            .map(|s| s.file.clone())
            .unwrap_or_default();
        let entry = CoveringTest {
            symbol: symbol.clone(),
            file,
            covers,
            gate_safe,
        };
        // A test the change itself modified is reported apart from the tests
        // that cover it: it is the thing whose behaviour changed, not evidence
        // that anything still works. So it is also not allowed to mark a
        // symbol covered — letting it would mean a diff that edits a function
        // and its only test reports full coverage, which is the one shape
        // where a reviewer most needs to be told something is missing.
        if changed.contains(&symbol) {
            modified_tests.push(entry);
        } else {
            if !gate_safe {
                structural_evidence += 1;
            }
            covered.extend(entry.covers.iter().cloned());
            tests.push(entry);
        }
    }

    tests.sort();
    modified_tests.sort();

    let uncovered: BTreeSet<SymbolId> = changed
        .iter()
        .filter(|id| !covered.contains(*id))
        // A changed test does not need covering by another test.
        .filter(|id| !modified_tests.iter().any(|t| &&t.symbol == id))
        .cloned()
        .collect();

    let mut gaps = Vec::new();
    let structural = crate::query::structural_files(graph).len();
    if structural > 0 {
        gaps.push(Gap::StructuralRegions(structural));
    }
    if !uncovered.is_empty() {
        gaps.push(Gap::UncoveredSymbols(uncovered.len()));
    }
    if structural_evidence > 0 {
        gaps.push(Gap::StructuralEvidence(structural_evidence));
    }
    gaps.sort();

    TestSelection {
        tests,
        modified_tests,
        uncovered,
        gaps,
    }
}

/// Every symbol from which one of `roots` can be reached, mapped back to the
/// roots it reaches.
///
/// Walks incoming edges, which is the same direction `blast_radius` walks: a
/// test that calls a function that calls the changed one is upstream of it.
/// `tests` edges are skipped during the walk — they are the answer, not a path
/// through the graph, and following them would let one test's coverage make
/// another test look like a caller.
fn reverse_reachable(
    graph: &InvarGraph,
    roots: &BTreeSet<SymbolId>,
    depth: usize,
    min_resolution: Resolution,
) -> BTreeMap<String, BTreeSet<SymbolId>> {
    let mut origins: BTreeMap<String, BTreeSet<SymbolId>> = BTreeMap::new();
    for root in roots {
        origins
            .entry(root.clone())
            .or_default()
            .insert(root.clone());
    }

    let mut frontier: Vec<SymbolId> = roots.iter().cloned().collect();
    for _ in 0..depth {
        let mut next: Vec<SymbolId> = Vec::new();
        for id in &frontier {
            let Some(node) = graph.node_index.get(id).map(|v| *v) else {
                continue;
            };
            let carried = origins.get(id.as_str()).cloned().unwrap_or_default();
            for edge in graph.graph.edges_directed(node, Direction::Incoming) {
                if edge.weight().kind == RelationshipKind::Tests {
                    continue;
                }
                if !edge.weight().resolution.meets(min_resolution) {
                    continue;
                }
                let Some(source) = graph.graph.node_weight(edge.source()) else {
                    continue;
                };
                let entry = origins.entry(source.clone()).or_default();
                let before = entry.len();
                entry.extend(carried.iter().cloned());
                // Enqueued only when it learned something, which is what stops
                // a cycle walking forever.
                if entry.len() != before {
                    next.push(source.clone());
                }
            }
        }
        if next.is_empty() {
            break;
        }
        next.sort();
        next.dedup();
        frontier = next;
    }

    origins
}
