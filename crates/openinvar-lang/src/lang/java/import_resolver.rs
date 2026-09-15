//! Java's name resolution, followed exactly.
//!
//! # Why the order matters
//!
//! Java resolves a simple name in a defined sequence, and it is not the same
//! as "look everywhere and take the first hit". A type declared in the file
//! shadows an import; the file's own package is searched before any on-demand
//! import; and two wildcard imports offering the same name is an *ambiguity*
//! the compiler rejects rather than a coin toss.
//!
//! Following that order is the whole value of this module. Matching leaf names
//! across the repository instead is the bug 0.2.0 fixed in the Rust adapter,
//! where use-paths matched anywhere in the tree.
//!
//! # Anything not resolved is dropped
//!
//! [`crate::dispatch::run_adapter`] stamps every Tier 1 edge `Resolution::Resolved`
//! after this returns, because a Tier 1 pipeline is *defined* as one that bound
//! its targets. So an edge left here half-resolved would be marked gate-safe
//! on its way out — a wrong answer a gate would then act on. The rule is
//! therefore absolute: resolve it, or do not emit it.
//!
//! What this drops is mostly the standard library. `String`, `List` and
//! `Object` are not in the repository, so no edge to them can be anything but
//! a guess, and a missing edge to `String` costs a user nothing.

use std::collections::{BTreeMap, BTreeSet};

use openinvar_core::ir::{FileIR, RelationshipKind, SymbolId, SymbolKind};
use openinvar_core::symbol_id::{
    external_package_id, is_placeholder, parse_unresolved_import_id,
    parse_unresolved_local_type_id,
};

use super::extractor::FileFacts;

/// Everything declared across the analysed files, indexed the ways resolution
/// needs to ask for it.
struct Index {
    /// `com.example.UserService` to its symbol id.
    by_fqn: BTreeMap<String, SymbolId>,
    /// `(file, SimpleName)` to its symbol id, for same-file shadowing.
    by_file: BTreeMap<(String, String), SymbolId>,
    /// Type symbol id to its members, by simple name.
    members: BTreeMap<SymbolId, BTreeMap<String, SymbolId>>,
    /// Type symbol id to the type ids it extends or implements.
    supertypes: BTreeMap<SymbolId, Vec<SymbolId>>,
    /// Type symbol id to the file that declares it.
    file_of: BTreeMap<SymbolId, String>,
}

const TYPE_KINDS: &[SymbolKind] = &[SymbolKind::Class, SymbolKind::Interface, SymbolKind::Enum];

fn build_index(files: &[FileIR], facts: &BTreeMap<String, FileFacts>) -> Index {
    let mut index = Index {
        by_fqn: BTreeMap::new(),
        by_file: BTreeMap::new(),
        members: BTreeMap::new(),
        supertypes: BTreeMap::new(),
        file_of: BTreeMap::new(),
    };

    for file_ir in files {
        let package = facts
            .get(&file_ir.file)
            .map(|f| f.package.clone())
            .unwrap_or_default();

        for symbol in &file_ir.symbols {
            if !TYPE_KINDS.contains(&symbol.kind) {
                continue;
            }
            let fqn = if package.is_empty() {
                symbol.name.clone()
            } else {
                format!("{package}.{}", symbol.name)
            };
            // First declaration wins, and files are sorted before analysis, so
            // a repository that genuinely declares one FQN twice resolves the
            // same way on every run rather than by scan order.
            index.by_fqn.entry(fqn).or_insert_with(|| symbol.id.clone());
            index
                .by_file
                .entry((file_ir.file.clone(), symbol.name.clone()))
                .or_insert_with(|| symbol.id.clone());
            index.file_of.insert(symbol.id.clone(), file_ir.file.clone());
        }

        // Members carry `Owner::member` in their id, which is how a member is
        // attached to its type without a second pass over the tree.
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
            let Some(owner_id) = index.by_file.get(&(file_ir.file.clone(), owner.to_string()))
            else {
                continue;
            };
            index
                .members
                .entry(owner_id.clone())
                .or_default()
                .insert(symbol.name.clone(), symbol.id.clone());
        }
    }

    index
}

/// Resolve every placeholder, dropping what cannot be resolved.
pub fn resolve(files: &mut [FileIR], facts: &[FileFacts]) {
    let facts: BTreeMap<String, FileFacts> =
        facts.iter().map(|f| (f.file.clone(), f.clone())).collect();
    let index = build_index(files, &facts);

    // Supertypes need the index, so they are filled once it exists.
    let mut supertypes: BTreeMap<SymbolId, Vec<SymbolId>> = BTreeMap::new();
    for file_ir in files.iter() {
        let Some(file_facts) = facts.get(&file_ir.file) else {
            continue;
        };
        for rel in &file_ir.relationships {
            if !matches!(rel.kind, RelationshipKind::Extends | RelationshipKind::Implements) {
                continue;
            }
            let Some(simple) = parse_unresolved_local_type_id(&rel.to) else {
                continue;
            };
            if let Some(target) = resolve_type(simple, file_facts, &index) {
                supertypes.entry(rel.from.clone()).or_default().push(target);
            }
        }
    }
    let index = Index { supertypes, ..index };

    for file_ir in files.iter_mut() {
        let Some(file_facts) = facts.get(&file_ir.file).cloned() else {
            // No facts means nothing in this file could be bound, so every
            // placeholder in it is a reference that went unplaced.
            let before = file_ir.relationships.len();
            file_ir.relationships.retain(|rel| !is_placeholder(&rel.to));
            file_ir.unbound_references = (before - file_ir.relationships.len()) as u32;
            continue;
        };

        // References this file named and this resolver could not place. Counted
        // where they are dropped, which is the only place that knows the
        // difference between "bound it" and "could not". What is emitted does
        // not change: an unresolved reference is still no edge.
        let mut unbound: u32 = 0;
        let mut resolved = Vec::with_capacity(file_ir.relationships.len());
        for mut rel in std::mem::take(&mut file_ir.relationships) {
            if !is_placeholder(&rel.to) {
                resolved.push(rel);
                continue;
            }

            // A type reference: extends, implements, a declared type, `new T()`.
            if let Some(simple) = parse_unresolved_local_type_id(&rel.to) {
                if let Some(target) = resolve_type(simple, &file_facts, &index) {
                    rel.to = target;
                    resolved.push(rel);
                } else {
                    unbound += 1;
                }
                continue;
            }

            // A member reference: an import, or a call on a typed receiver.
            if let Some((owner, member)) = parse_unresolved_import_id(&rel.to) {
                match rel.kind {
                    RelationshipKind::Imports => {
                        // The owner is a package here, so the pair is already
                        // a fully-qualified type name.
                        let fqn = format!("{owner}.{member}");
                        if let Some(target) = index.by_fqn.get(&fqn) {
                            rel.to = target.clone();
                            resolved.push(rel);
                        } else {
                            // Outside the repository — a jar, or the standard
                            // library. Recorded as an external package so the
                            // dependency is visible without inventing a symbol.
                            rel.to = external_package_id(owner);
                            resolved.push(rel);
                        }
                    }
                    _ => {
                        if let Some(target) =
                            resolve_member(owner, member, &file_facts, &index)
                        {
                            rel.to = target;
                            resolved.push(rel);
                        } else {
                            unbound += 1;
                        }
                    }
                }
                continue;
            }
        }
        file_ir.relationships = resolved;
        file_ir.unbound_references = unbound;
    }
}

/// Java's resolution order for a simple type name.
fn resolve_type(simple: &str, facts: &FileFacts, index: &Index) -> Option<SymbolId> {
    // 1. Declared in this file — shadows everything.
    if let Some(id) = index.by_file.get(&(facts.file.clone(), simple.to_string())) {
        return Some(id.clone());
    }

    // 2. A single-type import names it explicitly.
    if let Some((_, fqn)) = facts.imports.iter().find(|(name, _)| name == simple) {
        return index.by_fqn.get(fqn).cloned();
    }

    // 3. The file's own package, which needs no import.
    if !facts.package.is_empty() {
        if let Some(id) = index.by_fqn.get(&format!("{}.{simple}", facts.package)) {
            return Some(id.clone());
        }
    }

    // 4. On-demand imports. Two packages offering the same name is an
    //    ambiguity the compiler rejects, so this refuses to choose rather than
    //    picking the first — guessing here is how a rename gets attributed to
    //    the wrong class.
    let candidates: BTreeSet<&SymbolId> = facts
        .wildcard_packages
        .iter()
        .filter_map(|package| index.by_fqn.get(&format!("{package}.{simple}")))
        .collect();
    if candidates.len() == 1 {
        return candidates.into_iter().next().cloned();
    }

    // 5. Not in the repository. The standard library lives here, and so does
    //    anything from a jar.
    None
}

/// A member of a type: `service.handle()`, or a statically imported `log()`.
fn resolve_member(
    owner: &str,
    member: &str,
    facts: &FileFacts,
    index: &Index,
) -> Option<SymbolId> {
    if let Some(type_id) = resolve_type(owner, facts, index) {
        if let Some(found) = lookup_member(&type_id, member, index) {
            return Some(found);
        }
    }

    // A bare call may be a static import rather than a method of the enclosing
    // type. Checked second because a method declared on the type shadows one
    // imported statically.
    if let Some((_, declaring)) = facts
        .static_imports
        .iter()
        .find(|(name, _)| name == member)
    {
        if let Some(type_id) = index.by_fqn.get(declaring) {
            return lookup_member(type_id, member, index);
        }
    }

    None
}

/// A member of a type or of anything it inherits from.
///
/// The supertype walk is what makes an inherited method resolve to the symbol
/// that actually declares it, rather than being dropped. Bounded by a visited
/// set: a repository can describe a cycle even though `javac` would reject it,
/// and an analyzer that hangs on bad input is worse than one that gives up.
fn lookup_member(type_id: &SymbolId, member: &str, index: &Index) -> Option<SymbolId> {
    let mut visited: BTreeSet<SymbolId> = BTreeSet::new();
    let mut queue = vec![type_id.clone()];

    while let Some(current) = queue.pop() {
        if !visited.insert(current.clone()) {
            continue;
        }
        if let Some(found) = index.members.get(&current).and_then(|m| m.get(member)) {
            return Some(found.clone());
        }
        if let Some(parents) = index.supertypes.get(&current) {
            // Sorted, so a member found on two supertypes resolves the same
            // way on every run.
            let mut parents = parents.clone();
            parents.sort();
            queue.extend(parents);
        }
    }

    None
}
