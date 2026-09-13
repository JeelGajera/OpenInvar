//! A minimal working set for orienting an agent on a symbol.
//!
//! # What this is trading away
//!
//! An agent orienting itself on a symbol reads files. Reading `ir.rs` to learn
//! what `RepoIR` is costs the whole file, most of which is other types. This
//! emits the graph's answer instead: the symbol, what it depends on, what
//! depends on it, and a signature for each — the shape of the neighbourhood
//! rather than its contents.
//!
//! The trade is real and worth stating plainly. A signature tells you a
//! function exists and what it takes; it does not tell you what it does. This
//! is for the first question an agent asks — *what am I about to touch, and
//! what touches it* — and not a substitute for reading the code it is going to
//! change.
//!
//! # Why the token figure is an estimate
//!
//! OpenInvar vendors no tokenizer and is not going to: a tokenizer is
//! model-specific, and a number that changed with somebody's model would not
//! be reproducible, which is the property everything here rests on. The
//! reported cost is a byte-based estimate, and it says so wherever it is
//! printed. It is honest about direction and magnitude, not about the exact
//! figure a given model would charge.

use std::collections::{BTreeMap, BTreeSet};

use petgraph::visit::EdgeRef as _;
use petgraph::Direction;

use crate::graph::InvarGraph;
use crate::ir::{RelationshipKind, Resolution, Symbol, SymbolId};

/// Bytes per token, for the estimate.
///
/// Four is the widely used rule of thumb for English-and-code text. It is a
/// rule of thumb, which is why every surface that prints a figure derived from
/// it calls it an estimate.
const BYTES_PER_TOKEN: usize = 4;

/// Estimate the tokens a string costs.
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(BYTES_PER_TOKEN)
}

/// How a symbol reached the working set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// What was asked about.
    Subject,
    /// Something the subject depends on.
    Dependency,
    /// Something that depends on the subject.
    Dependent,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Subject => "subject",
            Self::Dependency => "dependency",
            Self::Dependent => "dependent",
        }
    }
}

/// One entry in the working set.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Entry {
    /// Ordered first so a sort puts subjects above their neighbourhood.
    pub role: Role,
    /// How many hops from the subject. Zero for the subject itself.
    pub hop: usize,
    pub file: String,
    pub line: u32,
    pub name: String,
    pub kind: String,
    /// The declaration, where the adapter recorded one.
    pub signature: Option<String>,
}

impl Entry {
    /// Whether this entry corresponds to a file on disk.
    ///
    /// An external package is a node in the graph with no file behind it.
    /// Counting one toward the comparison would try to read a path that does
    /// not exist and take the whole measurement down with it.
    pub fn is_on_disk(&self) -> bool {
        !self.file.is_empty()
    }

    /// The single line this renders as.
    pub fn render(&self) -> String {
        // An external package has no location worth printing.
        if !self.is_on_disk() {
            return format!("{}  (external package)", self.name);
        }
        // The synthetic per-file module symbol is named "module" in every
        // file, so printing the name twice says nothing. The path is the
        // information: it names the file that reaches the subject.
        if self.kind == "module" {
            return format!("{}  (module)", self.file);
        }
        match &self.signature {
            Some(signature) => format!(
                "{}:{}  {}  {}",
                self.file, self.line, self.kind, signature
            ),
            None => format!("{}:{}  {}  {}", self.file, self.line, self.kind, self.name),
        }
    }
}

/// A working set, and what it cost.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkingSet {
    pub entries: Vec<Entry>,
    /// Entries dropped to stay inside a budget.
    ///
    /// Reported rather than silently cut: a truncated working set that looks
    /// complete is worse than one that says what is missing, because an agent
    /// will reason from the absence.
    pub omitted: usize,
    /// Estimated tokens of the rendered output.
    pub estimated_tokens: usize,
    /// Files the entries came from, for the comparison figure.
    pub files: BTreeSet<String>,
    /// Whether every edge walked was resolved.
    pub gate_safe: bool,
}

impl WorkingSet {
    pub fn render(&self) -> String {
        let mut out = String::new();
        for entry in &self.entries {
            out.push_str(&entry.render());
            out.push('\n');
        }
        out
    }
}

/// Build a working set for `subjects`.
///
/// `depth` bounds both directions independently. `budget`, when given, caps
/// the estimated tokens; entries are dropped from the outermost hop inward, so
/// what survives is the part nearest what was asked about.
pub fn build(
    graph: &InvarGraph,
    subjects: &BTreeSet<SymbolId>,
    depth: usize,
    budget: Option<usize>,
    min_resolution: Resolution,
) -> WorkingSet {
    let mut entries: BTreeMap<SymbolId, Entry> = BTreeMap::new();
    let mut gate_safe = true;

    for id in subjects {
        if let Some(symbol) = graph.symbols.get(id) {
            entries.insert(id.clone(), entry_for(&symbol, Role::Subject, 0));
        }
    }

    for (direction, role) in [
        (Direction::Outgoing, Role::Dependency),
        (Direction::Incoming, Role::Dependent),
    ] {
        let mut frontier: Vec<SymbolId> = subjects.iter().cloned().collect();
        let mut seen: BTreeSet<SymbolId> = subjects.iter().cloned().collect();

        for hop in 1..=depth {
            let mut next = Vec::new();
            for id in &frontier {
                let Some(node) = graph.node_index.get(id).map(|v| *v) else {
                    continue;
                };
                for edge in graph.graph.edges_directed(node, direction) {
                    // A `tests` edge is derived from a call that is already
                    // here, so following it would list the same neighbour
                    // twice and spend budget on a duplicate.
                    if edge.weight().kind == RelationshipKind::Tests {
                        continue;
                    }
                    if !edge.weight().resolution.meets(min_resolution) {
                        continue;
                    }
                    if !edge.weight().resolution.is_gate_safe() {
                        gate_safe = false;
                    }
                    let target = match direction {
                        Direction::Outgoing => edge.target(),
                        Direction::Incoming => edge.source(),
                    };
                    let Some(neighbour) = graph.graph.node_weight(target) else {
                        continue;
                    };
                    if !seen.insert(neighbour.clone()) {
                        continue;
                    }
                    if let Some(symbol) = graph.symbols.get(neighbour) {
                        entries
                            .entry(neighbour.clone())
                            .or_insert_with(|| entry_for(&symbol, role, hop));
                    }
                    next.push(neighbour.clone());
                }
            }
            if next.is_empty() {
                break;
            }
            next.sort();
            next.dedup();
            frontier = next;
        }
    }

    // Sorted rather than left in walk order: the graph's edge order follows
    // insertion, and a working set that varied between runs would make two
    // identical questions cost different things.
    let mut ordered: Vec<Entry> = entries.into_values().collect();
    ordered.sort();

    // Drop the synthetic module row for a file that already contributed a real
    // symbol. Inbound edges to a type are dominated by imports, which adapters
    // attribute to the file's module symbol, so without this a working set is
    // mostly rows that say "this file mentions it" beside a row that says
    // which function does — and the second makes the first redundant.
    let files_with_real_symbols: BTreeSet<String> = ordered
        .iter()
        .filter(|e| e.kind != "module" && e.is_on_disk())
        .map(|e| e.file.clone())
        .collect();
    ordered.retain(|e| e.kind != "module" || !files_with_real_symbols.contains(&e.file));

    let mut omitted = 0;
    if let Some(budget) = budget {
        let mut kept = Vec::new();
        let mut spent = 0;
        for entry in ordered {
            let cost = estimate_tokens(&entry.render()) + 1;
            if spent + cost > budget {
                omitted += 1;
                continue;
            }
            spent += cost;
            kept.push(entry);
        }
        ordered = kept;
    }

    let files: BTreeSet<String> = ordered
        .iter()
        .filter(|e| e.is_on_disk())
        .map(|e| e.file.clone())
        .collect();
    let rendered: String = ordered
        .iter()
        .map(|e| format!("{}\n", e.render()))
        .collect();

    WorkingSet {
        estimated_tokens: estimate_tokens(&rendered),
        entries: ordered,
        omitted,
        files,
        gate_safe,
    }
}

fn entry_for(symbol: &Symbol, role: Role, hop: usize) -> Entry {
    Entry {
        role,
        hop,
        file: symbol.file.clone(),
        line: symbol.line_start,
        name: symbol.name.clone(),
        kind: format!("{:?}", symbol.kind).to_lowercase(),
        signature: symbol.signature.clone(),
    }
}

/// What reading the same files whole would have cost, estimated the same way.
///
/// The honest baseline: an agent orienting itself without a graph opens the
/// files these symbols live in and reads them entire. Measured from disk
/// rather than asserted.
///
/// Files that could not be read are counted and returned rather than folded
/// into the total, because a file silently treated as zero makes the saving
/// look larger than it is — and the saving is this feature's entire claim.
pub struct WholeFileCost {
    pub tokens: usize,
    pub files_read: usize,
    pub files_unreadable: usize,
}

pub fn whole_file_tokens(root: &std::path::Path, files: &BTreeSet<String>) -> WholeFileCost {
    let mut cost = WholeFileCost {
        tokens: 0,
        files_read: 0,
        files_unreadable: 0,
    };
    for file in files {
        match std::fs::read_to_string(root.join(file)) {
            Ok(text) => {
                cost.tokens += estimate_tokens(&text);
                cost.files_read += 1;
            }
            Err(_) => cost.files_unreadable += 1,
        }
    }
    cost
}
