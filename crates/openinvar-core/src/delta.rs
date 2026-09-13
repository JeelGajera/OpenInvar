//! What changed between two graphs.
//!
//! Every capability above this — broken-edge findings, rule evaluation, the
//! audit detectors — is a query over this delta, so its job is to describe the
//! change accurately and to say nothing it cannot support.
//!
//! # Why continuity is modelled rather than inferred
//!
//! A symbol's id embeds its file and its name, so renaming or moving one
//! produces a different id. Compared naively that reads as a delete plus an
//! add, and every consumer then reports a symbol destroyed and an unrelated
//! one created — the loudest possible description of the smallest possible
//! change. Worse, the later detectors would count a rename as contract
//! erosion.
//!
//! So a removed symbol and an added one are paired into a [`Continuity`] when
//! the evidence is strong enough, and the pairing says which of name or file
//! moved. Where the evidence is not strong enough they stay a separate removal
//! and addition, which is the honest description of "these might be the same
//! symbol and nothing here can tell".

use std::collections::{BTreeMap, BTreeSet};

use crate::graph::InvarGraph;
use crate::ir::{RelationshipKind, Resolution, Symbol};

/// One edge, reduced to the parts a delta compares.
///
/// Deliberately not the whole `RelationshipMeta`: two runs may record the same
/// edge with a different `context` string when surrounding source moved, and a
/// delta that reported those as changes would drown the ones that matter.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EdgeRef {
    pub from: String,
    pub to: String,
    pub kind: RelationshipKind,
    pub file: String,
    pub line: u32,
    pub resolution: Resolution,
}

/// How a symbol that survived the change was identified across it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Continuation {
    /// Same file, different name.
    Renamed,
    /// Same name, different file.
    Moved,
    /// Both, which is the weakest evidence and needs a matching signature.
    RenamedAndMoved,
}

/// A symbol present on both sides under different ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Continuity {
    pub before: Symbol,
    pub after: Symbol,
    pub how: Continuation,
}

/// A symbol whose id is unchanged but whose signature is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureChange {
    pub before: Symbol,
    pub after: Symbol,
}

/// The difference between two graphs.
///
/// Every field is ordered, so the same pair of graphs always produces the same
/// delta — the property the whole product rests on, and the one a `HashMap`
/// anywhere in here would quietly destroy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphDelta {
    pub added_symbols: Vec<Symbol>,
    pub removed_symbols: Vec<Symbol>,
    pub continuities: Vec<Continuity>,
    pub signature_changes: Vec<SignatureChange>,
    pub added_edges: Vec<EdgeRef>,
    pub removed_edges: Vec<EdgeRef>,
}

impl GraphDelta {
    /// Whether the two graphs are indistinguishable to this comparison.
    pub fn is_empty(&self) -> bool {
        self.added_symbols.is_empty()
            && self.removed_symbols.is_empty()
            && self.continuities.is_empty()
            && self.signature_changes.is_empty()
            && self.added_edges.is_empty()
            && self.removed_edges.is_empty()
    }
}

fn symbols_by_id(graph: &InvarGraph) -> BTreeMap<String, Symbol> {
    graph
        .symbols
        .iter()
        .map(|entry| (entry.key().clone(), entry.value().clone()))
        .collect()
}

fn edges(graph: &InvarGraph) -> BTreeSet<EdgeRef> {
    let mut out = BTreeSet::new();
    for index in graph.graph.edge_indices() {
        let Some((source, target)) = graph.graph.edge_endpoints(index) else {
            continue;
        };
        let (Some(from), Some(to), Some(meta)) = (
            graph.graph.node_weight(source),
            graph.graph.node_weight(target),
            graph.graph.edge_weight(index),
        ) else {
            continue;
        };
        out.insert(EdgeRef {
            from: from.clone(),
            to: to.clone(),
            kind: meta.kind.clone(),
            file: meta.file.clone(),
            line: meta.line,
            resolution: meta.resolution,
        });
    }
    out
}

/// A signature worth matching on.
///
/// An absent or empty signature is not evidence of anything: every symbol
/// without one would match every other, so pairing on it would invent
/// continuities wholesale.
fn usable_signature(symbol: &Symbol) -> Option<&str> {
    symbol
        .signature
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Whether `before` becomes `after` under nothing but a rename.
///
/// A signature is the symbol's source text, so a rename changes it — the
/// declaration contains the name. Comparing them raw therefore never matches a
/// genuine rename, and comparing only position matches anything that happens
/// to sit on the same line, which is exactly what replacing one function with
/// another looks like.
///
/// Substituting the old name for the new one separates the two precisely:
///
/// ```text
/// class UserPayload { id: string }  ->  class CustomerPayload { id: string }
///   substituted: class CustomerPayload { id: string }        matches, a rename
///
/// function doomed() { return 2 }    ->  function freshlyAdded() { return 3 }
///   substituted: function freshlyAdded() { return 2 }        differs, not one
/// ```
///
/// A rename that also edits the body records no continuity, which is the
/// honest answer: the evidence for "same symbol" is gone, and inventing a
/// rename is worse than missing one.
fn renames_to(before: &Symbol, after: &Symbol) -> bool {
    let (Some(before_sig), Some(after_sig)) = (usable_signature(before), usable_signature(after))
    else {
        return false;
    };
    if before.name.is_empty() {
        return false;
    }
    before_sig.replace(&before.name, &after.name) == after_sig
}

/// How a removed and an added symbol relate, if they are the same symbol.
///
/// Kind must always match: a function becoming a class of the same name is not
/// one symbol that moved, and treating it as one would hide a real change.
fn continuation_between(before: &Symbol, after: &Symbol) -> Option<Continuation> {
    if before.kind != after.kind || before.language != after.language {
        return None;
    }

    let same_name = before.name == after.name;
    let same_file = before.file == after.file;
    match (same_name, same_file) {
        // Same id would not have reached here.
        (true, true) => None,
        // Moved: the name is the evidence, and a name is a strong hint within
        // one repository even without a signature.
        (true, false) => Some(Continuation::Moved),
        // Renamed in place. The signature must match once the old name is
        // substituted for the new: a rename changes the source text, so raw
        // equality never holds, and position alone pairs any two symbols that
        // happen to share a line — which is what replacing one function with
        // another looks like.
        (false, true) => renames_to(before, after).then_some(Continuation::Renamed),
        // Both changed, so the substituted signature is the only evidence.
        (false, false) => renames_to(before, after).then_some(Continuation::RenamedAndMoved),
    }
}

/// Rank a candidate pairing; lower is better.
///
/// Used to make the choice deterministic when a removed symbol could pair with
/// several added ones, and to prefer the strongest available evidence.
/// `signature_matches` means the substituted signatures agree — see
/// [`renames_to`].
fn pairing_rank(how: Continuation, signature_matches: bool) -> u8 {
    match (how, signature_matches) {
        (Continuation::Renamed, true) => 0,
        (Continuation::Moved, true) => 1,
        (Continuation::RenamedAndMoved, true) => 2,
        (Continuation::Renamed, false) => 3,
        (Continuation::Moved, false) => 4,
        (Continuation::RenamedAndMoved, false) => 5,
    }
}

/// Compare two graphs.
pub fn compute(before: &InvarGraph, after: &InvarGraph) -> GraphDelta {
    let before_symbols = symbols_by_id(before);
    let after_symbols = symbols_by_id(after);

    let mut removed: Vec<Symbol> = Vec::new();
    let mut added: Vec<Symbol> = Vec::new();
    let mut signature_changes = Vec::new();

    for (id, symbol) in &before_symbols {
        match after_symbols.get(id) {
            None => removed.push(symbol.clone()),
            Some(current) if current.signature != symbol.signature => {
                signature_changes.push(SignatureChange {
                    before: symbol.clone(),
                    after: current.clone(),
                });
            }
            Some(_) => {}
        }
    }
    for (id, symbol) in &after_symbols {
        if !before_symbols.contains_key(id) {
            added.push(symbol.clone());
        }
    }

    // ── pair removals with additions ─────────────────────────
    //
    // Greedy over a fully ordered candidate list rather than optimal matching.
    // An optimal assignment would be more accurate in contrived cases and less
    // predictable in every case, and predictability is the property being
    // bought here: the same two graphs must always produce the same pairs.
    let mut candidates: Vec<(u8, String, String, Continuation)> = Vec::new();
    for old in &removed {
        for new in &added {
            if let Some(how) = continuation_between(old, new) {
                // The same predicate the pairing used, so ranking reflects the
                // evidence that admitted the pair rather than a weaker one. For
                // a move the substitution is a no-op, so this reduces to plain
                // signature equality there.
                candidates.push((
                    pairing_rank(how, renames_to(old, new)),
                    old.id.clone(),
                    new.id.clone(),
                    how,
                ));
            }
        }
    }
    candidates.sort();

    let mut paired_before: BTreeSet<String> = BTreeSet::new();
    let mut paired_after: BTreeSet<String> = BTreeSet::new();
    let mut continuities = Vec::new();

    for (_, old_id, new_id, how) in candidates {
        if paired_before.contains(&old_id) || paired_after.contains(&new_id) {
            continue;
        }
        let (Some(before_symbol), Some(after_symbol)) =
            (before_symbols.get(&old_id), after_symbols.get(&new_id))
        else {
            continue;
        };
        paired_before.insert(old_id);
        paired_after.insert(new_id);
        continuities.push(Continuity {
            before: before_symbol.clone(),
            after: after_symbol.clone(),
            how,
        });
    }

    removed.retain(|s| !paired_before.contains(&s.id));
    added.retain(|s| !paired_after.contains(&s.id));

    let before_edges = edges(before);
    let after_edges = edges(after);

    let mut delta = GraphDelta {
        added_symbols: added,
        removed_symbols: removed,
        continuities,
        signature_changes,
        added_edges: after_edges.difference(&before_edges).cloned().collect(),
        removed_edges: before_edges.difference(&after_edges).cloned().collect(),
    };

    delta.added_symbols.sort_by(|a, b| a.id.cmp(&b.id));
    delta.removed_symbols.sort_by(|a, b| a.id.cmp(&b.id));
    delta
        .continuities
        .sort_by(|a, b| a.before.id.cmp(&b.before.id));
    delta
        .signature_changes
        .sort_by(|a, b| a.before.id.cmp(&b.before.id));
    delta
}

/// Symbols removed outright, as ids, for callers checking broken references.
pub fn removed_symbol_ids(delta: &GraphDelta) -> BTreeSet<String> {
    delta
        .removed_symbols
        .iter()
        .map(|s| s.id.clone())
        .collect()
}

/// Whether a delta touches anything a gate may act on.
///
/// A change made entirely of structural edges is invisible to a gate, and a
/// caller that treats "no resolved changes" as "no changes" would pass a Tier 2
/// repository unconditionally.
pub fn has_resolved_changes(delta: &GraphDelta) -> bool {
    delta
        .added_edges
        .iter()
        .chain(delta.removed_edges.iter())
        .any(|e| e.resolution == Resolution::Resolved)
        || !delta.added_symbols.is_empty()
        || !delta.removed_symbols.is_empty()
}

/// The kinds present in a delta's edges, for reporting what a result covers.
pub fn edge_kinds(delta: &GraphDelta) -> BTreeSet<RelationshipKind> {
    delta
        .added_edges
        .iter()
        .chain(delta.removed_edges.iter())
        .map(|e| e.kind.clone())
        .collect()
}
