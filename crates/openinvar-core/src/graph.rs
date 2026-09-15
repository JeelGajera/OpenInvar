use dashmap::DashMap;
use petgraph::graph::{DiGraph, NodeIndex};

use crate::ir::{ReExportEntry, Relationship, RelationshipKind, Resolution, Symbol, SymbolId};
use crate::resolver::AliasEntry;

#[derive(Debug, Clone)]
pub struct RelationshipMeta {
    pub kind: RelationshipKind,
    /// How `to` was determined. See [`crate::ir::Resolution`].
    pub resolution: Resolution,
    pub alias: Option<String>,
    pub properties_accessed: Vec<String>,
    pub context: String,
    pub file: String,
    pub line: u32,
}

#[derive(Debug)]
pub struct InvarGraph {
    pub graph: DiGraph<SymbolId, RelationshipMeta>,
    pub node_index: DashMap<SymbolId, NodeIndex>,
    pub name_index: DashMap<String, Vec<SymbolId>>,
    pub file_index: DashMap<String, Vec<SymbolId>>,
    pub symbols: DashMap<SymbolId, Symbol>,
    pub alias_chains: DashMap<SymbolId, Vec<AliasEntry>>,
    pub file_reexports: DashMap<String, Vec<ReExportEntry>>,
    /// Recognised assertions per symbol, for the symbols that have any.
    ///
    /// A side map rather than a field on [`Symbol`], following `alias_chains`
    /// and `file_reexports`: the same per-symbol data that not every symbol
    /// carries, kept where adding it does not touch every construction site.
    ///
    /// Only non-zero counts are held. **Absent means zero only in a counted
    /// language** — see `assertions_counted` for the languages this build can
    /// see, because "no assertions left" and "cannot see the assertions" are
    /// opposite conclusions and only the first is a finding.
    pub assertions: DashMap<SymbolId, u32>,
    /// Files whose language this build counts assertions in.
    ///
    /// Carried alongside the counts because the counts alone cannot say
    /// whether an absent entry is a zero or an unknown. Supplied by the caller
    /// for the same reason tier is: it is a property of the language adapter,
    /// which this crate deliberately does not depend on.
    pub assertions_counted: DashMap<String, bool>,
    /// References each file named that its adapter could not place.
    ///
    /// Keyed by file, like `assertions_counted`, and present only for files a
    /// Tier 1 adapter analysed. This is the denominator resolution coverage
    /// does not have: an unbound reference produces no edge, so it cannot be
    /// counted from the graph's edges afterwards — it has to be carried here
    /// from the analysis that saw it.
    pub unbound_references: DashMap<String, u32>,
}

impl Default for InvarGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl InvarGraph {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            node_index: DashMap::new(),
            name_index: DashMap::new(),
            file_index: DashMap::new(),
            symbols: DashMap::new(),
            alias_chains: DashMap::new(),
            file_reexports: DashMap::new(),
            assertions: DashMap::new(),
            assertions_counted: DashMap::new(),
            unbound_references: DashMap::new(),
        }
    }

    /// How many symbols have at least one alias.
    pub fn aliased_symbol_count(&self) -> usize {
        self.alias_chains.len()
    }

    /// How many aliases exist in total.
    ///
    /// `alias_chains` is keyed by symbol, so its length counts symbols that
    /// have aliases, not aliases. Reported as "Alias chains", that read as a
    /// count of renames and was wrong whenever one symbol was renamed more
    /// than once: a type imported under a different name by twelve files
    /// reported `1`.
    pub fn alias_count(&self) -> usize {
        self.alias_chains.iter().map(|e| e.value().len()).sum()
    }

    pub fn add_symbol(&mut self, symbol: Symbol) {
        if self.node_index.contains_key(&symbol.id) {
            self.replace_symbol(symbol);
            return;
        }

        let symbol_id = symbol.id.clone();
        let symbol_name = symbol.name.clone();
        let file = symbol.file.clone();

        let node = self.graph.add_node(symbol_id.clone());
        self.node_index.insert(symbol_id.clone(), node);
        self.symbols.insert(symbol_id.clone(), symbol);

        self.name_index
            .entry(symbol_name)
            .and_modify(|ids| {
                ids.push(symbol_id.clone());
                ids.sort();
                ids.dedup();
            })
            .or_insert_with(|| vec![symbol_id.clone()]);

        self.file_index
            .entry(file)
            .and_modify(|ids| {
                ids.push(symbol_id.clone());
                ids.sort();
                ids.dedup();
            })
            .or_insert_with(|| vec![symbol_id]);
    }

    pub fn replace_symbol(&mut self, symbol: Symbol) {
        let symbol_id = symbol.id.clone();
        if let Some(existing) = self.symbols.get(&symbol_id) {
            let existing_name = existing.name.clone();
            let existing_file = existing.file.clone();
            drop(existing);

            if existing_name != symbol.name {
                if let Some(mut ids) = self.name_index.get_mut(&existing_name) {
                    ids.retain(|id| id != &symbol_id);
                }
                self.name_index
                    .entry(symbol.name.clone())
                    .and_modify(|ids| {
                        ids.push(symbol_id.clone());
                        ids.sort();
                        ids.dedup();
                    })
                    .or_insert_with(|| vec![symbol_id.clone()]);
            }

            if existing_file != symbol.file {
                if let Some(mut ids) = self.file_index.get_mut(&existing_file) {
                    ids.retain(|id| id != &symbol_id);
                }
                self.file_index
                    .entry(symbol.file.clone())
                    .and_modify(|ids| {
                        ids.push(symbol_id.clone());
                        ids.sort();
                        ids.dedup();
                    })
                    .or_insert_with(|| vec![symbol_id.clone()]);
            }
        }

        self.symbols.insert(symbol_id, symbol);
    }

    pub fn add_relationship(&mut self, relationship: &Relationship) {
        let Some(from) = self.node_index.get(&relationship.from).map(|v| *v) else {
            return;
        };

        // Auto-create external package nodes on first reference
        if !self.node_index.contains_key(&relationship.to) && relationship.to.starts_with("ext::") {
            let package_name = relationship
                .to
                .strip_prefix("ext::")
                .and_then(|s| s.strip_suffix("::package"))
                .unwrap_or(&relationship.to)
                .to_string();
            self.add_symbol(crate::ir::Symbol {
                id: relationship.to.clone(),
                name: package_name,
                kind: crate::ir::SymbolKind::ExternalPackage,
                language: crate::ir::Language::TypeScript,
                file: String::new(),
                line_start: 0,
                line_end: 0,
                signature: None,
            });
        }

        let Some(to) = self.node_index.get(&relationship.to).map(|v| *v) else {
            return;
        };

        let meta = RelationshipMeta {
            kind: relationship.kind.clone(),
            resolution: relationship.resolution,
            alias: relationship.alias.clone(),
            properties_accessed: relationship.properties_accessed.clone(),
            context: relationship.context.clone(),
            file: relationship.file.clone(),
            line: relationship.line,
        };
        self.graph.add_edge(from, to, meta);
    }

    pub fn remove_relationships_in_file(&mut self, file: &str) -> usize {
        let edge_ids: Vec<_> = self
            .graph
            .edge_indices()
            .filter(|edge_id| {
                self.graph
                    .edge_weight(*edge_id)
                    .map(|meta| meta.file == file)
                    .unwrap_or(false)
            })
            .collect();

        let removed = edge_ids.len();
        for edge_id in edge_ids {
            let _ = self.graph.remove_edge(edge_id);
        }
        removed
    }

    pub fn remove_file(&mut self, file: &str) -> Vec<SymbolId> {
        let mut removed = Vec::new();
        let symbol_ids = self
            .file_index
            .remove(file)
            .map(|(_, ids)| ids)
            .unwrap_or_default();

        for symbol_id in &symbol_ids {
            if let Some((_, symbol)) = self.symbols.remove(symbol_id) {
                if let Some(mut ids) = self.name_index.get_mut(&symbol.name) {
                    ids.retain(|id| id != symbol_id);
                }
            }
            if let Some((_, node)) = self.node_index.remove(symbol_id) {
                let _ = self.graph.remove_node(node);
            }
            self.alias_chains.remove(symbol_id);
            removed.push(symbol_id.clone());
        }
        self.file_reexports.remove(file);

        self.rebuild_node_index();
        removed.sort();
        removed
    }

    fn rebuild_node_index(&self) {
        self.node_index.clear();
        for node_index in self.graph.node_indices() {
            if let Some(symbol_id) = self.graph.node_weight(node_index) {
                self.node_index.insert(symbol_id.clone(), node_index);
            }
        }
    }
}
