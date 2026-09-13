use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

use petgraph::visit::EdgeRef;
use petgraph::Direction;

use crate::error::InvarError;
use crate::graph::InvarGraph;
use crate::index::find_symbol_id;
use crate::ir::{RelationshipKind, Resolution, SymbolId};

const DEFAULT_DEPTH: usize = 3;
const MAX_DEPTH: usize = 10;

/// Every kind of relationship, in a fixed order.
///
/// The order is the enum's own declaration order and is what `--kind help` and
/// any listing render, so it must not depend on how a `match` happens to be
/// written.
pub const ALL_KINDS: [RelationshipKind; 9] = [
    RelationshipKind::Imports,
    RelationshipKind::Calls,
    RelationshipKind::Extends,
    RelationshipKind::Implements,
    RelationshipKind::UsesType,
    RelationshipKind::AccessesProperty,
    RelationshipKind::ReExports,
    RelationshipKind::Instantiates,
    RelationshipKind::Tests,
];

/// Files whose edges were resolved structurally, i.e. within one file only.
///
/// An empty blast radius is a claim about the whole repository. Structural
/// analysis cannot see across files, so a reference from such a file to the
/// symbol in question would not appear in the graph at all — the emptiness is
/// partly an artefact of how much was resolved, not a fact about the code.
/// Callers use this to qualify the verdict rather than withdraw it: the answer
/// is still the best available, it is just not one a gate may act on alone.
pub fn structural_files(graph: &InvarGraph) -> BTreeSet<String> {
    graph
        .graph
        .edge_references()
        .filter(|e| !e.weight().resolution.is_gate_safe())
        .map(|e| e.weight().file.clone())
        .collect()
}

/// Whether every edge in this graph was resolved well enough to gate on.
///
/// A graph with no edges at all is not gate-safe: there is nothing to have
/// been resolved, so an empty result from it carries no evidence either way.
pub fn is_gate_safe(graph: &InvarGraph) -> bool {
    let mut any = false;
    for e in graph.graph.edge_references() {
        any = true;
        if !e.weight().resolution.is_gate_safe() {
            return false;
        }
    }
    any
}

/// The relationship kinds this graph actually contains.
///
/// A filter that can only ever match nothing is a trap in a tool meant to gate
/// changes: a rule scoped to such a kind would never fire and would read as a
/// pass. Callers use this to say so rather than returning a silent empty
/// result.
///
/// This asks the graph in hand rather than consulting a hand-maintained list
/// of unimplemented kinds. That list was wrong twice inside a single release —
/// once when structural analysis began emitting `Calls`, and again when call
/// edges shipped for some languages but not others. A constant cannot express
/// "this repository is Python, and call edges are a TypeScript feature so far",
/// but the graph can, because the answer is simply which edges are in it.
pub fn kinds_present(graph: &crate::graph::InvarGraph) -> BTreeSet<RelationshipKind> {
    graph
        .graph
        .edge_references()
        .map(|e| e.weight().kind.clone())
        .collect()
}

/// The name a kind is known by on the command line and in JSON.
pub fn kind_name(kind: &RelationshipKind) -> &'static str {
    match kind {
        RelationshipKind::Imports => "imports",
        RelationshipKind::Calls => "calls",
        RelationshipKind::Extends => "extends",
        RelationshipKind::Implements => "implements",
        RelationshipKind::UsesType => "uses-type",
        RelationshipKind::AccessesProperty => "accesses-property",
        RelationshipKind::ReExports => "re-exports",
        RelationshipKind::Instantiates => "instantiates",
        RelationshipKind::Tests => "tests",
    }
}

/// Parse a kind from its command-line name.
pub fn parse_kind(name: &str) -> Option<RelationshipKind> {
    ALL_KINDS
        .iter()
        .find(|k| kind_name(k) == name)
        .map(|k| (*k).clone())
}

/// Which relationship kinds a traversal should follow.
///
/// `RelationshipMeta` has always carried the kind of every edge, and
/// `traverse` has always discarded it, so a query could not tell a test's
/// reference from an import from a trait implementation. Every gate and
/// finding in the later phases needs that distinction, and the data was
/// already there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelationshipKindMask(u16);

impl Default for RelationshipKindMask {
    fn default() -> Self {
        Self::all()
    }
}

impl RelationshipKindMask {
    /// Follow every kind — what traversal did before it could filter.
    pub fn all() -> Self {
        Self(u16::MAX)
    }

    pub fn none() -> Self {
        Self(0)
    }

    pub fn from_kinds(kinds: &[RelationshipKind]) -> Self {
        kinds
            .iter()
            .fold(Self::none(), |mask, kind| mask.with(kind.clone()))
    }

    pub fn with(self, kind: RelationshipKind) -> Self {
        Self(self.0 | (1 << Self::bit(&kind)))
    }

    pub fn contains(&self, kind: &RelationshipKind) -> bool {
        self.0 & (1 << Self::bit(kind)) != 0
    }

    /// True when the mask admits everything, so callers can skip reporting a
    /// filter the user did not ask for.
    pub fn is_all(&self) -> bool {
        ALL_KINDS.iter().all(|k| self.contains(k))
    }

    pub fn is_empty(&self) -> bool {
        ALL_KINDS.iter().all(|k| !self.contains(k))
    }

    /// The kinds in this mask, in [`ALL_KINDS`] order.
    pub fn kinds(&self) -> Vec<RelationshipKind> {
        ALL_KINDS
            .iter()
            .filter(|k| self.contains(k))
            .cloned()
            .collect()
    }

    fn bit(kind: &RelationshipKind) -> u16 {
        match kind {
            RelationshipKind::Imports => 0,
            RelationshipKind::Calls => 1,
            RelationshipKind::Extends => 2,
            RelationshipKind::Implements => 3,
            RelationshipKind::UsesType => 4,
            RelationshipKind::AccessesProperty => 5,
            RelationshipKind::ReExports => 6,
            RelationshipKind::Instantiates => 7,
            RelationshipKind::Tests => 8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryEdge {
    pub from: SymbolId,
    pub to: SymbolId,
    /// What kind of reference this edge records.
    ///
    /// Carried on `RelationshipMeta` since 0.2.0 and dropped at the query
    /// boundary until now, which left every consumer treating a property
    /// access and a trait implementation as the same fact.
    pub kind: RelationshipKind,
    /// How this edge's target was determined.
    ///
    /// A result set mixes tiers in a polyglot repository, so the caller needs
    /// this per row rather than per query to know which rows a gate may act on.
    pub resolution: Resolution,
    pub file: String,
    pub line: u32,
    pub alias: Option<String>,
    pub properties_accessed: Vec<String>,
    pub context: String,
    pub hop: usize,
}

/// Split edges into those that reach the symbol under its own name and those
/// that rename it.
///
/// The distinction drives the "HIGH RISK" labelling in both the CLI and the MCP
/// server: a caller that imports `UserPayload as ResponseModel` will not turn up
/// in a text search for the original name, which is exactly the case OpenInvar
/// exists to surface.
///
/// An `alias` equal to the symbol's own name is not a rename. Adapters record
/// the local name on type-reference edges whether or not it differs, so testing
/// `alias.is_some()` alone flagged ordinary same-name references as high risk
/// and buried the genuine renames among them.
pub fn partition_by_alias<'e>(
    graph: &InvarGraph,
    edges: &'e [QueryEdge],
) -> (Vec<&'e QueryEdge>, Vec<&'e QueryEdge>) {
    let mut direct = Vec::new();
    let mut aliased = Vec::new();

    for edge in edges {
        if is_renamed(graph, edge) {
            aliased.push(edge);
        } else {
            direct.push(edge);
        }
    }

    (direct, aliased)
}

/// True when `edge` refers to its target under a name other than the target's.
pub fn is_renamed(graph: &InvarGraph, edge: &QueryEdge) -> bool {
    let Some(alias) = edge.alias.as_deref() else {
        return false;
    };
    match graph.symbols.get(&edge.to) {
        // A qualified reference such as `models.UserPayload` names the symbol
        // directly; only the final segment is compared.
        Some(symbol) => alias.rsplit(['.', ':']).next().unwrap_or(alias) != symbol.name,
        // Unknown target: an alias is the only name we have, so treat it as one.
        None => true,
    }
}

/// True when every reference that touches `property` reaches its type under a
/// different name.
///
/// Such a property is invisible to a text search for the declaring type, which
/// is the case worth flagging. One that is also read under the type's own name
/// somewhere is not.
///
/// Two things this got wrong before. It tested `alias.is_some()`, the same
/// mistake `partition_by_alias` was fixed for — adapters record the local name
/// on a reference whether or not it differs, so an ordinary same-name access
/// was labelled aliased. And `all` over an empty iterator is true, so a
/// property no edge actually touches was labelled aliased as well.
pub fn is_aliased_only_property(graph: &InvarGraph, edges: &[QueryEdge], property: &str) -> bool {
    let mut touching = edges
        .iter()
        .filter(|e| e.properties_accessed.iter().any(|p| p == property))
        .peekable();

    if touching.peek().is_none() {
        return false;
    }
    touching.all(|e| is_renamed(graph, e))
}

pub fn blast_radius(
    graph: &InvarGraph,
    symbol: &str,
    file: Option<&str>,
    depth: Option<usize>,
    kinds: RelationshipKindMask,
    min_resolution: Resolution,
) -> Result<Vec<QueryEdge>, InvarError> {
    let effective_depth = depth.unwrap_or(DEFAULT_DEPTH);
    if effective_depth > MAX_DEPTH {
        return Err(InvarError::InvalidDepth {
            depth: effective_depth,
            max: MAX_DEPTH,
        });
    }

    let root = find_symbol_id(graph, symbol, file)?;
    traverse(graph, &root, effective_depth, Direction::Incoming, kinds, min_resolution)
}

pub fn dependencies(
    graph: &InvarGraph,
    symbol: &str,
    file: Option<&str>,
    depth: Option<usize>,
    kinds: RelationshipKindMask,
    min_resolution: Resolution,
) -> Result<Vec<QueryEdge>, InvarError> {
    let effective_depth = depth.unwrap_or(DEFAULT_DEPTH);
    if effective_depth > MAX_DEPTH {
        return Err(InvarError::InvalidDepth {
            depth: effective_depth,
            max: MAX_DEPTH,
        });
    }

    let root = find_symbol_id(graph, symbol, file)?;
    traverse(graph, &root, effective_depth, Direction::Outgoing, kinds, min_resolution)
}

pub fn symbol_usages(
    graph: &InvarGraph,
    symbol: &str,
    file: Option<&str>,
    include_aliases: bool,
    kinds: RelationshipKindMask,
    min_resolution: Resolution,
) -> Result<Vec<QueryEdge>, InvarError> {
    let root = find_symbol_id(graph, symbol, file)?;
    let mut results = traverse(graph, &root, 1, Direction::Incoming, kinds, min_resolution)?;

    if include_aliases {
        if let Some(aliases) = graph.alias_chains.get(&root) {
            let alias_set: HashSet<String> = aliases.iter().map(|a| a.alias_name.clone()).collect();
            for edge in &mut results {
                if edge.alias.is_none() && edge.context.contains(" as ") {
                    let alias = edge
                        .context
                        .split(" as ")
                        .nth(1)
                        .and_then(|v| v.split_whitespace().next())
                        .map(|s| s.trim_matches(|c: char| c == ',' || c == ';').to_string());
                    if let Some(found) = alias {
                        if alias_set.contains(&found) {
                            edge.alias = Some(found);
                        }
                    }
                }
            }
        }
    } else {
        results.retain(|edge| edge.alias.is_none());
    }

    dedupe_edges(results)
}

/// Walk outward from `root`, following only edges whose kind is in `kinds`.
///
/// Filtering happens on the edge itself rather than on the collected result,
/// so an excluded kind also stops the walk continuing through it. Reaching a
/// symbol only via a kind the caller excluded means it is not reachable under
/// that question: asking "what imports this?" should not return something that
/// merely inherits from an importer.
fn traverse(
    graph: &InvarGraph,
    root: &SymbolId,
    max_depth: usize,
    direction: Direction,
    kinds: RelationshipKindMask,
    min_resolution: Resolution,
) -> Result<Vec<QueryEdge>, InvarError> {
    let Some(root_node) = graph.node_index.get(root).map(|v| *v) else {
        return Err(InvarError::SymbolNotFound(root.clone()));
    };

    let mut queue = VecDeque::new();
    let mut visited = HashSet::new();
    let mut results = Vec::new();

    queue.push_back((root_node, 0usize));
    visited.insert(root_node);

    while let Some((node, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        for edge in graph.graph.edges_directed(node, direction) {
            if !kinds.contains(&edge.weight().kind) {
                continue;
            }
            // Below the threshold the edge is dropped *and* not walked through.
            // Filtering the result set afterwards would keep every hop reached
            // by way of an edge the caller said it could not trust, which is
            // the opposite of what asking for a threshold means.
            if !edge.weight().resolution.meets(min_resolution) {
                continue;
            }
            let neighbor = if direction == Direction::Incoming {
                edge.source()
            } else {
                edge.target()
            };

            let from_id = graph
                .graph
                .node_weight(edge.source())
                .cloned()
                .ok_or_else(|| InvarError::GraphCorrupt("Missing source node".to_string()))?;
            let to_id = graph
                .graph
                .node_weight(edge.target())
                .cloned()
                .ok_or_else(|| InvarError::GraphCorrupt("Missing target node".to_string()))?;
            let meta = edge.weight();

            results.push(QueryEdge {
                from: from_id,
                to: to_id,
                kind: meta.kind.clone(),
                resolution: meta.resolution,
                file: meta.file.clone(),
                line: meta.line,
                alias: meta.alias.clone(),
                properties_accessed: meta.properties_accessed.clone(),
                context: meta.context.clone(),
                hop: depth + 1,
            });

            if visited.insert(neighbor) {
                queue.push_back((neighbor, depth + 1));
            }
        }
    }

    dedupe_edges(results)
}

/// Collapse rows that describe the same fact, then order them.
///
/// One reference produces several edges when the source location is
/// attributed at more than one level — a class-level edge and a method-level
/// edge to the same target at the same line — and the old key included
/// `from`, so all of them survived as separate rows. A 64-file project
/// reported 196 "dependents" for 38 referencing files, and the aliased
/// findings that are the whole point of the tool sat below 160 rows of
/// duplicates.
///
/// The identity of a reference is what a reader can act on: which symbol,
/// where, under what name, by what kind of reference. `from` is not part of
/// that — the location already says where — so it no longer splits rows.
/// `kind` is, because an `extends` and a field read at one line are two
/// different facts about the code.
///
/// Sorting happens before collapsing rather than after, so which row survives
/// is decided by the ordering rather than by traversal order. The lowest hop
/// wins, which is the shortest path to the symbol and the one worth showing.
fn dedupe_edges(mut edges: Vec<QueryEdge>) -> Result<Vec<QueryEdge>, InvarError> {
    edges.sort_by(|a, b| {
        a.hop
            .cmp(&b.hop)
            .then(a.file.cmp(&b.file))
            .then(a.line.cmp(&b.line))
            .then(a.to.cmp(&b.to))
            .then(kind_name(&a.kind).cmp(kind_name(&b.kind)))
            .then(a.alias.cmp(&b.alias))
            .then(a.from.cmp(&b.from))
    });

    let mut seen = BTreeSet::new();
    edges.retain(|edge| {
        seen.insert((
            edge.to.clone(),
            edge.file.clone(),
            edge.line,
            edge.alias.clone(),
            kind_name(&edge.kind),
        ))
    });

    Ok(edges)
}

// ── file-level impact ────────────────────────────────────────

/// What depends on the symbols defined in one file.
///
/// The unit an editing agent actually works in. `blast_radius` answers a
/// question about a symbol, but a hook firing before a write knows only which
/// file is about to change — it has no symbol to ask about yet, and asking
/// about the wrong one is worse than not asking.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileImpact {
    /// The file as the graph records it, repository-relative.
    pub file: String,
    /// Symbols this file defines, by id, in a stable order.
    pub defined: Vec<SymbolId>,
    /// Every edge reaching into the file from outside it.
    pub edges: Vec<QueryEdge>,
    /// Files that reach in, and how many edges each contributes.
    pub dependents: BTreeMap<String, usize>,
    /// Whether every edge here was resolved.
    ///
    /// A hook reporting "nothing depends on this file" on structural evidence
    /// would be making the claim OpenInvar exists to refuse.
    pub gate_safe: bool,
    /// Files analyzed within-file only, whose references could not reach this
    /// answer even if they exist.
    pub blind_spots: usize,
}

impl FileImpact {
    /// Whether anything outside the file depends on it.
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }
}

/// Compute [`FileImpact`] for `file`, which must be as the graph records it.
///
/// Edges originating inside the same file are dropped: a hook is warning about
/// what a change reaches beyond the file being edited, and a file's references
/// to itself are already in front of the person editing it.
///
/// An unknown file is not an error. A new file, or one no adapter handles, has
/// no symbols and therefore no dependents — and a hook that fails on the first
/// untracked path is a hook that gets switched off.
pub fn file_impact(
    graph: &InvarGraph,
    file: &str,
    depth: Option<usize>,
    kinds: RelationshipKindMask,
    min_resolution: Resolution,
) -> Result<FileImpact, InvarError> {
    let effective_depth = depth.unwrap_or(DEFAULT_DEPTH);
    if effective_depth > MAX_DEPTH {
        return Err(InvarError::InvalidDepth {
            depth: effective_depth,
            max: MAX_DEPTH,
        });
    }

    let mut defined: Vec<SymbolId> = graph
        .file_index
        .get(file)
        .map(|ids| ids.clone())
        .unwrap_or_default();
    defined.sort();

    // Deduplicated across roots: two symbols in this file can share a
    // dependent, and reporting it twice would inflate every count a reader
    // uses to judge how risky the edit is.
    let mut seen: HashSet<(SymbolId, SymbolId, usize)> = HashSet::new();
    let mut edges: Vec<QueryEdge> = Vec::new();

    for root in &defined {
        let found = match traverse(
            graph,
            root,
            effective_depth,
            Direction::Incoming,
            kinds,
            min_resolution,
        ) {
            Ok(found) => found,
            // A symbol in the file index with no node is a graph the store
            // wrote inconsistently, not a reason to fail the whole answer.
            Err(InvarError::SymbolNotFound(_)) => continue,
            Err(err) => return Err(err),
        };
        for edge in found {
            if edge.file == file {
                continue;
            }
            if seen.insert((edge.from.clone(), edge.to.clone(), edge.line as usize)) {
                edges.push(edge);
            }
        }
    }

    // Sorted explicitly: `file_index` and the traversal both walk structures
    // whose order is not guaranteed between runs.
    edges.sort_by(|a, b| {
        (&a.file, a.line, &a.from, &a.to).cmp(&(&b.file, b.line, &b.from, &b.to))
    });

    let mut dependents: BTreeMap<String, usize> = BTreeMap::new();
    for edge in &edges {
        *dependents.entry(edge.file.clone()).or_default() += 1;
    }

    let gate_safe = edges.iter().all(|e| e.resolution.is_gate_safe());

    Ok(FileImpact {
        file: file.to_string(),
        defined,
        edges,
        dependents,
        gate_safe,
        blind_spots: structural_files(graph).len(),
    })
}
