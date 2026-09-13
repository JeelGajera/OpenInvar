use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use crate::graph::InvarGraph;
use crate::ir::{Relationship, RelationshipKind, SymbolId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum AliasScope {
    ImportAlias,
    ReExport,
    BarrelReExport,
    DefaultImport,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct AliasEntry {
    pub alias_name: String,
    pub defined_in_file: String,
    pub scope: AliasScope,
}

#[derive(Default)]
pub struct AliasResolver {
    alias_to_canonical: DashMap<String, SymbolId>,
}

impl AliasResolver {
    pub fn ingest_relationships(&self, graph: &InvarGraph, relationships: &[Relationship]) {
        for relationship in relationships {
            if relationship.kind == RelationshipKind::Imports
                || relationship.kind == RelationshipKind::ReExports
            {
                if let Some(alias) = &relationship.alias {
                    let scope = infer_scope(relationship, alias);
                    self.alias_to_canonical.insert(
                        make_alias_key(&relationship.file, alias),
                        relationship.to.clone(),
                    );
                    graph
                        .alias_chains
                        .entry(relationship.to.clone())
                        .and_modify(|entries| {
                            entries.push(AliasEntry {
                                alias_name: alias.clone(),
                                defined_in_file: relationship.file.clone(),
                                scope: scope.clone(),
                            });
                            entries.sort_by(|a, b| {
                                a.defined_in_file
                                    .cmp(&b.defined_in_file)
                                    .then(a.alias_name.cmp(&b.alias_name))
                            });
                            entries.dedup();
                        })
                        .or_insert_with(|| {
                            vec![AliasEntry {
                                alias_name: alias.clone(),
                                defined_in_file: relationship.file.clone(),
                                scope,
                            }]
                        });
                }
            }
        }
    }

    pub fn canonicalize_relationship(&self, relationship: &Relationship) -> Relationship {
        if relationship.kind == RelationshipKind::AccessesProperty {
            if let Some(alias) = &relationship.alias {
                if let Some(canonical_to) = self.resolve_alias_in_file(alias, &relationship.file) {
                    let mut normalized = relationship.clone();
                    normalized.to = canonical_to;
                    return normalized;
                }
            }
        }

        relationship.clone()
    }

    pub fn canonicalize_relationships(&self, relationships: &[Relationship]) -> Vec<Relationship> {
        relationships
            .iter()
            .map(|relationship| self.canonicalize_relationship(relationship))
            .collect()
    }

    pub fn resolve_alias_in_file(&self, alias: &str, file: &str) -> Option<SymbolId> {
        self.alias_to_canonical
            .get(&make_alias_key(file, alias))
            .map(|v| v.value().clone())
    }
}

fn infer_scope(relationship: &Relationship, alias: &str) -> AliasScope {
    if relationship.kind == RelationshipKind::ReExports {
        if relationship.context.contains("export *") {
            return AliasScope::BarrelReExport;
        }
        return AliasScope::ReExport;
    }
    if relationship.context.contains("default") {
        return AliasScope::DefaultImport;
    }
    if relationship.context.contains(" as ") || relationship.to.rsplit("::").next() != Some(alias) {
        return AliasScope::ImportAlias;
    }
    AliasScope::ImportAlias
}

fn make_alias_key(file: &str, alias: &str) -> String {
    format!("{file}::{alias}")
}
