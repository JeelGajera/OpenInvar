//! C#'s name resolution, followed in its own order.
//!
//! # Three things C# does that Java does not
//!
//! **`using` names a namespace, not a type.** There is no single-type import;
//! `using App.Models` brings in everything in that namespace at once. So the
//! ambiguity that Java only reaches through wildcard imports is C#'s *ordinary*
//! case, and refusing to guess between two candidates matters more here.
//!
//! **Enclosing namespaces are in scope.** Inside `namespace A.B.C`, a type in
//! `A.B` or `A` is visible with no `using` at all, and the innermost match
//! wins. A resolver that only searched the exact namespace would drop most
//! references in any deeply-namespaced project.
//!
//! **A type can be declared in several files.** `partial class Report` in two
//! files is *one* type with the union of their members, so a call to a method
//! declared in the other half has to resolve. Indexing members by the owner's
//! fully-qualified name rather than by declaration is what makes that fall out
//! rather than need special-casing.
//!
//! # Anything not resolved is dropped
//!
//! [`crate::dispatch::run_adapter`] stamps every Tier 1 edge
//! `Resolution::Resolved` after this returns. An edge left half-resolved would
//! be marked gate-safe on its way out — a wrong answer a gate would act on. So:
//! resolve it, or do not emit it. What that drops is mostly the framework,
//! which no repository here declares.

use std::collections::{BTreeMap, BTreeSet};

use openinvar_core::ir::{FileIR, RelationshipKind, SymbolId, SymbolKind};
use openinvar_core::symbol_id::{
    external_package_id, is_placeholder, parse_unresolved_import_id,
    parse_unresolved_local_type_id, IMPORT_ALL,
};

use super::extractor::{simple_name, FileFacts};

struct Index {
    /// `App.Services.Report` to the symbol id of its canonical declaration.
    ///
    /// Canonical because a partial type has several. The first in sorted file
    /// order wins, which makes the choice the same on every run.
    by_fqn: BTreeMap<String, SymbolId>,
    /// Owner FQN to its members, merged across every declaration of a partial
    /// type.
    members: BTreeMap<String, BTreeMap<String, SymbolId>>,
    /// Type FQN to the FQNs it derives from.
    bases: BTreeMap<String, Vec<String>>,
    /// Every namespace that declares at least one type, for resolving a
    /// `using` to something real rather than to a guess.
    namespaces: BTreeSet<String>,
}

const TYPE_KINDS: &[SymbolKind] = &[SymbolKind::Class, SymbolKind::Interface, SymbolKind::Enum];

fn fqn(namespace: &str, name: &str) -> String {
    if namespace.is_empty() {
        name.to_string()
    } else {
        format!("{namespace}.{name}")
    }
}

fn build_index(files: &[FileIR], facts: &BTreeMap<String, FileFacts>) -> Index {
    let mut index = Index {
        by_fqn: BTreeMap::new(),
        members: BTreeMap::new(),
        bases: BTreeMap::new(),
        namespaces: BTreeSet::new(),
    };

    for file_ir in files {
        let Some(file_facts) = facts.get(&file_ir.file) else {
            continue;
        };
        let namespace_of: BTreeMap<String, String> =
            file_facts.declared.iter().cloned().collect();

        for symbol in &file_ir.symbols {
            if !TYPE_KINDS.contains(&symbol.kind) {
                continue;
            }
            let namespace = namespace_of.get(&symbol.name).cloned().unwrap_or_default();
            index.namespaces.insert(namespace.clone());
            index
                .by_fqn
                .entry(fqn(&namespace, &symbol.name))
                .or_insert_with(|| symbol.id.clone());
        }

        // Members are attached to the owner's *fully-qualified name*, not to a
        // declaration id. Two halves of a partial class then land in one entry
        // without the resolver having to know partials exist.
        for symbol in &file_ir.symbols {
            if !matches!(symbol.kind, SymbolKind::Method | SymbolKind::Property) {
                continue;
            }
            let Some((_, qualified, _)) = openinvar_core::symbol_id::parse_symbol_id(&symbol.id)
            else {
                continue;
            };
            let Some((owner, _)) = qualified.split_once("::") else {
                continue;
            };
            let namespace = namespace_of.get(owner).cloned().unwrap_or_default();
            index
                .members
                .entry(fqn(&namespace, owner))
                .or_default()
                .insert(symbol.name.clone(), symbol.id.clone());
        }
    }

    index
}

pub fn resolve(files: &mut [FileIR], facts: &[FileFacts]) {
    let facts: BTreeMap<String, FileFacts> =
        facts.iter().map(|f| (f.file.clone(), f.clone())).collect();
    let mut index = build_index(files, &facts);

    // Base types need the index, so they are filled once it exists.
    let mut bases: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for file_ir in files.iter() {
        let Some(file_facts) = facts.get(&file_ir.file) else {
            continue;
        };
        let namespace_of: BTreeMap<String, String> =
            file_facts.declared.iter().cloned().collect();
        let owner_fqn: BTreeMap<&str, String> = file_ir
            .symbols
            .iter()
            .filter(|s| TYPE_KINDS.contains(&s.kind))
            .map(|s| {
                (
                    s.id.as_str(),
                    fqn(
                        namespace_of.get(&s.name).map(String::as_str).unwrap_or(""),
                        &s.name,
                    ),
                )
            })
            .collect();

        for rel in &file_ir.relationships {
            if rel.kind != RelationshipKind::Implements {
                continue;
            }
            let (Some(simple), Some(from)) = (
                parse_unresolved_local_type_id(&rel.to),
                owner_fqn.get(rel.from.as_str()),
            ) else {
                continue;
            };
            if let Some(target) = resolve_fqn(simple, file_facts, &index) {
                bases.entry(from.clone()).or_default().push(target);
            }
        }
    }
    index.bases = bases;

    for file_ir in files.iter_mut() {
        let Some(file_facts) = facts.get(&file_ir.file).cloned() else {
            // No facts means nothing in this file could be bound, so every
            // placeholder in it is a reference that went unplaced.
            let before = file_ir.relationships.len();
            file_ir.relationships.retain(|rel| !is_placeholder(&rel.to));
            file_ir.unbound_references = (before - file_ir.relationships.len()) as u32;
            file_ir.references_counted = true;
            continue;
        };

        // References this file named and this resolver could not place. Counted
        // where they are dropped, which is the only place that knows the
        // difference between "bound it" and "could not". Nothing about what is
        // emitted changes — an unresolved reference is still no edge.
        let mut unbound: u32 = 0;
        let mut resolved = Vec::with_capacity(file_ir.relationships.len());
        for mut rel in std::mem::take(&mut file_ir.relationships) {
            if !is_placeholder(&rel.to) {
                resolved.push(rel);
                continue;
            }

            if let Some(simple) = parse_unresolved_local_type_id(&rel.to) {
                if let Some(target) = resolve_type(simple, &file_facts, &index) {
                    rel.to = target;
                    resolved.push(rel);
                } else {
                    unbound += 1;
                }
                continue;
            }

            if let Some((owner, member)) = parse_unresolved_import_id(&rel.to) {
                if rel.kind == RelationshipKind::Imports && member == IMPORT_ALL {
                    // `using App.Models` names a namespace, and a namespace is
                    // not a symbol — there is nothing in the graph to point at.
                    //
                    // When the repository declares that namespace, the using is
                    // not a dependency on anything outside it, and the concrete
                    // edges to the types it brought in already carry the real
                    // information. Recording it as an "external package" would
                    // put the repository's own code in the third-party list.
                    // So it is dropped.
                    //
                    // When the repository does not declare it — `using System`,
                    // a NuGet package — it is a real outside dependency, and an
                    // external package node makes that visible without
                    // inventing a symbol.
                    if !index.namespaces.contains(owner) {
                        rel.to = external_package_id(owner);
                        resolved.push(rel);
                    }
                    continue;
                }
                if let Some(target) = resolve_member(owner, member, &file_facts, &index) {
                    rel.to = target;
                    resolved.push(rel);
                } else {
                    unbound += 1;
                }
            }
        }
        file_ir.relationships = resolved;
        file_ir.unbound_references = unbound;
        file_ir.references_counted = true;
    }
}

/// The fully-qualified name a simple name refers to in this file.
fn resolve_fqn(simple: &str, facts: &FileFacts, index: &Index) -> Option<String> {
    // 1. An alias is an explicit naming and beats everything it competes with.
    if let Some((_, target)) = facts.aliases.iter().find(|(name, _)| name == simple) {
        if index.by_fqn.contains_key(target) {
            return Some(target.clone());
        }
        // An alias may name a type outside the repository.
        return None;
    }

    // 2. This file's own declarations, and then every enclosing namespace,
    //    innermost first. Inside `A.B.C` a type in `A.B` is visible with no
    //    using at all, and the nearer declaration wins rather than being
    //    ambiguous with the further one.
    let mut scopes: BTreeSet<String> = facts.declared.iter().map(|(_, ns)| ns.clone()).collect();
    for namespace in facts.declared.iter().map(|(_, ns)| ns.clone()).collect::<Vec<_>>() {
        let mut current = namespace.as_str();
        while let Some((parent, _)) = current.rsplit_once('.') {
            scopes.insert(parent.to_string());
            current = parent;
        }
    }
    let mut ordered: Vec<&String> = scopes.iter().collect();
    // Longest first is innermost first.
    ordered.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
    for namespace in ordered {
        let candidate = fqn(namespace, simple);
        if index.by_fqn.contains_key(&candidate) {
            return Some(candidate);
        }
    }

    // 3. `using` directives, which name namespaces and are all equal. Two
    //    offering the name is the ambiguity the compiler rejects, so this
    //    refuses rather than choosing.
    let candidates: BTreeSet<String> = facts
        .usings
        .iter()
        .map(|namespace| fqn(namespace, simple))
        .filter(|candidate| index.by_fqn.contains_key(candidate))
        .collect();
    if candidates.len() == 1 {
        return candidates.into_iter().next();
    }

    // 4. The global namespace.
    if index.by_fqn.contains_key(simple) {
        return Some(simple.to_string());
    }

    None
}

fn resolve_type(simple: &str, facts: &FileFacts, index: &Index) -> Option<SymbolId> {
    resolve_fqn(simple, facts, index).and_then(|name| index.by_fqn.get(&name).cloned())
}

fn resolve_member(
    owner: &str,
    member: &str,
    facts: &FileFacts,
    index: &Index,
) -> Option<SymbolId> {
    if let Some(owner_fqn) = resolve_fqn(owner, facts, index) {
        if let Some(found) = lookup_member(&owner_fqn, member, index) {
            return Some(found);
        }
    }

    // A bare call may come from `using static`, which is checked second: a
    // member declared on the enclosing type shadows an imported one.
    for static_type in &facts.static_usings {
        if let Some(found) = lookup_member(static_type, member, index) {
            return Some(found);
        }
        // The directive may name the type by a path this repository spells
        // differently; fall back to its simple name resolved as usual.
        if let Some(resolved) = resolve_fqn(&simple_name(static_type), facts, index) {
            if let Some(found) = lookup_member(&resolved, member, index) {
                return Some(found);
            }
        }
    }

    None
}

/// A member of a type or of anything it derives from.
///
/// Bounded by a visited set: a repository can describe a cycle even though the
/// compiler would reject it, and an analyzer that hangs on bad input is worse
/// than one that gives up.
fn lookup_member(owner_fqn: &str, member: &str, index: &Index) -> Option<SymbolId> {
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut queue = vec![owner_fqn.to_string()];

    while let Some(current) = queue.pop() {
        if !visited.insert(current.clone()) {
            continue;
        }
        if let Some(found) = index.members.get(&current).and_then(|m| m.get(member)) {
            return Some(found.clone());
        }
        if let Some(parents) = index.bases.get(&current) {
            let mut parents = parents.clone();
            parents.sort();
            queue.extend(parents);
        }
    }

    None
}
