//! Turning a delta into judgements.
//!
//! [`crate::delta`] says what changed. This says what about the change is
//! worth someone's attention, and it is the layer every gate in the later
//! phases reads: a rule violation, an audit finding and a CI comment are all
//! phrased in terms of these.
//!
//! # Every finding carries its own confidence
//!
//! A finding derived entirely from structural edges is advisory, and one
//! derived from resolved edges is a fact. Mixing them into a single list with
//! no distinction is how a gate ends up blocking a change on a guess, so each
//! finding records the weakest resolution among the evidence that produced it
//! and callers filter on that rather than on the repository's average.

use std::collections::{BTreeMap, BTreeSet};

use petgraph::visit::EdgeRef;

use crate::coverage::Coverage;
use crate::delta::GraphDelta;
use crate::graph::InvarGraph;
use crate::ir::{RelationshipKind, Resolution, Symbol, SymbolKind};

/// What kind of problem a finding describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FindingKind {
    /// A symbol was removed while something that still exists referred to it.
    BrokenEdge,
    /// A symbol survived, but everything that referred to it stopped.
    Orphaned,
    /// A symbol that forms part of an API surface was removed outright.
    ApiSurfaceRemoved,
    /// A symbol kept its identity and changed its signature.
    SignatureChanged,
}

impl FindingKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BrokenEdge => "broken-edge",
            Self::Orphaned => "orphaned",
            Self::ApiSurfaceRemoved => "api-surface-removed",
            Self::SignatureChanged => "signature-changed",
        }
    }
}

/// Something that referred to the symbol a finding is about.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Referrer {
    pub symbol: String,
    pub file: String,
    pub line: u32,
    pub kind: RelationshipKind,
    pub resolution: Resolution,
}

/// One judgement about a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub kind: FindingKind,
    pub symbol: String,
    pub name: String,
    pub file: String,
    pub line: u32,
    /// Who referred to it. Empty for findings that are not about references.
    pub referrers: Vec<Referrer>,
    /// The weakest resolution among the evidence.
    ///
    /// Weakest rather than strongest deliberately: a finding is only as
    /// trustworthy as the shakiest step in the reasoning that produced it.
    pub resolution: Resolution,
}

/// Findings for one delta, with the context needed to read them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffFindings {
    pub findings: Vec<Finding>,
    /// Resolution coverage over the files the change touched.
    ///
    /// Repository-wide coverage is the wrong denominator for a diff: a change
    /// confined to one badly resolved file is not made trustworthy by the rest
    /// of the repository resolving well.
    pub changed_file_coverage: Coverage,
    /// Every symbol the change can reach, by way of resolved edges only.
    pub blast_radius: BTreeSet<String>,
}

impl DiffFindings {
    pub fn of_kind(&self, kind: FindingKind) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(move |f| f.kind == kind)
    }

    /// Findings a gate may act on.
    pub fn gate_safe(&self) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(|f| f.resolution == Resolution::Resolved)
    }
}

/// Symbol kinds that form an API surface.
///
/// A local variable disappearing is refactoring; a method or an enum variant
/// disappearing is a contract change for whoever depended on it. Modules and
/// external packages are excluded: a module node is an artefact of how files
/// are represented rather than something a caller depends on by name.
fn is_api_surface(kind: &SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Class
            | SymbolKind::Interface
            | SymbolKind::TypeAlias
            | SymbolKind::Function
            | SymbolKind::Method
            | SymbolKind::Property
            | SymbolKind::Enum
            | SymbolKind::EnumVariant
    )
}

fn inbound(graph: &InvarGraph, id: &str) -> Vec<Referrer> {
    let Some(node) = graph.node_index.get(id).map(|v| *v) else {
        return Vec::new();
    };
    let mut out: Vec<Referrer> = graph
        .graph
        .edges_directed(node, petgraph::Direction::Incoming)
        .filter_map(|edge| {
            let source = graph.graph.node_weight(edge.source())?;
            let meta = edge.weight();
            Some(Referrer {
                symbol: source.clone(),
                file: meta.file.clone(),
                line: meta.line,
                kind: meta.kind.clone(),
                resolution: meta.resolution,
            })
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

fn weakest(referrers: &[Referrer]) -> Resolution {
    if referrers
        .iter()
        .any(|r| r.resolution == Resolution::Structural)
        || referrers.is_empty()
    {
        // No evidence is not strong evidence. A finding with no referrers is
        // reported at the advisory level so a gate does not act on silence.
        return Resolution::Structural;
    }
    Resolution::Resolved
}

/// Files whose own symbols the author edited.
///
/// Distinct from [`changed_files`], and the distinction is load-bearing. A
/// file that merely lost an edge is usually *reacting* to a change elsewhere —
/// deleting a function removes the call edges pointing at it, and those edges
/// are recorded in the caller's file without the caller having been touched.
/// Counting that as an edit would make every caller look edited, which is
/// exactly backwards for deciding whether a caller was left behind.
///
/// A file whose symbols were added, removed, renamed or re-signed is one
/// somebody actually opened.
fn edited_files(delta: &GraphDelta) -> BTreeSet<String> {
    let mut files = BTreeSet::new();
    for symbol in delta.added_symbols.iter().chain(delta.removed_symbols.iter()) {
        files.insert(symbol.file.clone());
    }
    for change in &delta.signature_changes {
        files.insert(change.before.file.clone());
        files.insert(change.after.file.clone());
    }
    for continuity in &delta.continuities {
        files.insert(continuity.before.file.clone());
        files.insert(continuity.after.file.clone());
    }
    files
}

/// Files the change touched at all, including by consequence.
///
/// The right denominator for coverage — a diff's trustworthiness depends on
/// every file its edges live in, edited or not.
fn changed_files(delta: &GraphDelta) -> BTreeSet<String> {
    let mut files = BTreeSet::new();
    for symbol in delta.added_symbols.iter().chain(delta.removed_symbols.iter()) {
        files.insert(symbol.file.clone());
    }
    for change in &delta.signature_changes {
        files.insert(change.after.file.clone());
    }
    for continuity in &delta.continuities {
        files.insert(continuity.before.file.clone());
        files.insert(continuity.after.file.clone());
    }
    for edge in delta.added_edges.iter().chain(delta.removed_edges.iter()) {
        files.insert(edge.file.clone());
    }
    files
}

fn coverage_over(graph: &InvarGraph, files: &BTreeSet<String>) -> Coverage {
    let mut coverage = Coverage::default();
    for (file, file_coverage) in crate::coverage::by_file(graph) {
        if files.contains(&file) {
            coverage.resolved += file_coverage.resolved;
            coverage.structural += file_coverage.structural;
        }
    }
    coverage
}

fn finding_for(kind: FindingKind, symbol: &Symbol, referrers: Vec<Referrer>) -> Finding {
    let resolution = match kind {
        // A signature change is read off the two symbols directly rather than
        // from any edge, so it is a fact regardless of what refers to it.
        FindingKind::SignatureChanged | FindingKind::ApiSurfaceRemoved => Resolution::Resolved,
        _ => weakest(&referrers),
    };
    Finding {
        kind,
        symbol: symbol.id.clone(),
        name: symbol.name.clone(),
        file: symbol.file.clone(),
        line: symbol.line_start,
        referrers,
        resolution,
    }
}

/// Derive findings from a delta and the two graphs it came from.
pub fn derive(before: &InvarGraph, after: &InvarGraph, delta: &GraphDelta) -> DiffFindings {
    let after_symbols: BTreeSet<String> = after
        .symbols
        .iter()
        .map(|entry| entry.key().clone())
        .collect();
    let edited = edited_files(delta);

    let mut findings = Vec::new();
    let mut blast_radius = BTreeSet::new();

    for removed in &delta.removed_symbols {
        // Who pointed at it before it went, narrowed twice.
        //
        // First: referrers that went too. A symbol and its only caller deleted
        // together is a clean removal, and reporting it as breakage would bury
        // the ones that are.
        //
        // Second, and less obvious: referrers in files the author edited. A
        // reference that no longer resolves produces no edge at all in the new
        // graph — it does not become a dangling one — so a surviving referrer
        // is not evidence that the reference survived. An edited caller
        // usually means the author updated it in the same change, which is the
        // opposite of breakage. What remains is a removal whose callers were
        // left alone, which is the thing worth reporting.
        //
        // The cost is a real false negative, and it is worth stating exactly:
        // a caller edited anywhere in the same change hides a stale reference
        // elsewhere in that same file. Rename a type in `app.ts` and delete a
        // function `app.ts` still calls, and this reports nothing.
        //
        // Closing it needs the analyzer's own "unable to resolve" diagnostics,
        // which say directly that a reference did not bind. Those exist per
        // file but are neither carried on the graph nor persisted in a
        // snapshot, so reaching them is a format change rather than a better
        // heuristic — deliberately not smuggled into this change.
        //
        // Until then the false negative is the right direction to fail: a gate
        // that cries wolf is disabled in week two, and an advisory finding on
        // every removal whose caller was correctly updated would be exactly
        // that.
        let surviving: Vec<Referrer> = inbound(before, &removed.id)
            .into_iter()
            .filter(|r| after_symbols.contains(&r.symbol))
            .filter(|r| !edited.contains(&r.file))
            .collect();

        for referrer in &surviving {
            if referrer.resolution == Resolution::Resolved {
                blast_radius.insert(referrer.symbol.clone());
            }
        }

        if !surviving.is_empty() {
            findings.push(finding_for(
                FindingKind::BrokenEdge,
                removed,
                surviving.clone(),
            ));
        }

        if is_api_surface(&removed.kind) {
            findings.push(finding_for(FindingKind::ApiSurfaceRemoved, removed, surviving));
        }
    }

    // Orphans: still present, but nothing refers to them any more.
    let mut inbound_before: BTreeMap<String, usize> = BTreeMap::new();
    for entry in before.symbols.iter() {
        inbound_before.insert(entry.key().clone(), inbound(before, entry.key()).len());
    }
    for entry in after.symbols.iter() {
        let id = entry.key();
        let symbol = entry.value();
        if symbol.kind == SymbolKind::Module {
            continue;
        }
        let had = inbound_before.get(id).copied().unwrap_or(0);
        if had > 0 && inbound(after, id).is_empty() {
            findings.push(Finding {
                kind: FindingKind::Orphaned,
                symbol: id.clone(),
                name: symbol.name.clone(),
                file: symbol.file.clone(),
                line: symbol.line_start,
                referrers: Vec::new(),
                // The evidence is the edges that used to exist; a symbol whose
                // only inbound edges were structural is an advisory orphan.
                resolution: weakest(&inbound(before, id)),
            });
        }
    }

    for change in &delta.signature_changes {
        let referrers: Vec<Referrer> = inbound(after, &change.after.id);
        for referrer in &referrers {
            if referrer.resolution == Resolution::Resolved {
                blast_radius.insert(referrer.symbol.clone());
            }
        }
        findings.push(finding_for(
            FindingKind::SignatureChanged,
            &change.after,
            referrers,
        ));
    }

    findings.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.symbol.cmp(&b.symbol))
            .then_with(|| a.line.cmp(&b.line))
    });

    DiffFindings {
        findings,
        changed_file_coverage: coverage_over(after, &changed_files(delta)),
        blast_radius,
    }
}
