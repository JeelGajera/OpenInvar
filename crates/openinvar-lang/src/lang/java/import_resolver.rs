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

use openinvar_core::ir::{FileIR, RelationshipKind, Symbol, SymbolId, SymbolKind};
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

        let canonical = canonical_names(file_ir);
        for symbol in &file_ir.symbols {
            if !TYPE_KINDS.contains(&symbol.kind) {
                continue;
            }
            // `Outer.Inner` for a nested type, the simple name for a top-level
            // one. Indexing a nested class under its simple name spells a name
            // no import can name, and misses every reference through the real
            // one.
            let name = canonical
                .get(&symbol.id)
                .cloned()
                .unwrap_or_else(|| symbol.name.clone());
            let fqn = if package.is_empty() {
                name
            } else {
                format!("{package}.{name}")
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

    // Every package some file in this tree declares. Used to tell a reference
    // that is genuinely outside the repository from one this resolver simply
    // did not index — see `is_outside_repository`.
    let repo_packages: BTreeSet<String> = facts
        .values()
        .filter(|f| !f.package.is_empty())
        .map(|f| f.package.clone())
        .collect();

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
        // The share of `unbound` this adapter can show is outside the tree.
        // Only ever incremented on positive evidence, so the remainder is an
        // over-estimate of what was missed rather than an under-estimate.
        let mut outside: u32 = 0;
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
                    if type_is_outside(simple, &file_facts, &repo_packages) {
                        outside += 1;
                    }
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
                            if member_is_outside(owner, member, &file_facts, &repo_packages)
                            {
                                outside += 1;
                            }
                        }
                    }
                }
                continue;
            }
        }
        file_ir.relationships = resolved;
        file_ir.unbound_references = unbound;
        file_ir.unbound_outside_repository = outside;
    }
}

/// The name each type in one file is imported by, keyed by symbol id.
///
/// Java's canonical name for a nested class is `Outer.Inner`, and that is what
/// an `import` names: `import com.google.gson.common.TestTypes.BagOfPrimitives`
/// is how gson reaches one. Indexing every type under its *simple* name put
/// that class at `com.google.gson.common.BagOfPrimitives` — a name no import
/// can spell — so the import missed, and so did all 235 references through it.
///
/// The nesting is recorded nowhere in the IR: a nested class's symbol id
/// carries its simple name and nothing else. It is recovered here from line
/// ranges instead, which the extractor already emits. A type declared inside
/// another is spanned by it, and two siblings never span each other, so the
/// innermost container is the immediate parent.
///
/// A local class — `void f() { class Tmp {} }` — comes out as
/// `Outer.Tmp`. Java gives those no importable name at all, so nothing can
/// reference one from another file and the entry is inert either way.
fn canonical_names(file_ir: &FileIR) -> BTreeMap<SymbolId, String> {
    let types: Vec<&Symbol> = file_ir
        .symbols
        .iter()
        .filter(|s| TYPE_KINDS.contains(&s.kind))
        .collect();

    let mut out = BTreeMap::new();
    for symbol in &types {
        let mut chain = vec![symbol.name.clone()];
        let mut current: &Symbol = symbol;
        // Walk outward. Every step moves to a strictly wider span, so this
        // terminates even if two declarations were to report the same range.
        while let Some(parent) = types
            .iter()
            .filter(|candidate| {
                candidate.id != current.id
                    && candidate.line_start <= current.line_start
                    && current.line_end <= candidate.line_end
                    && (candidate.line_start < current.line_start
                        || current.line_end < candidate.line_end)
            })
            .max_by_key(|c| (c.line_start, std::cmp::Reverse(c.line_end)))
        {
            chain.push(parent.name.clone());
            current = parent;
        }
        chain.reverse();
        out.insert(symbol.id.clone(), chain.join("."));
    }
    out
}

/// Type names `java.lang` provides, which need no import.
///
/// Every entry here is a claim that the name is outside any repository, so a
/// wrong one silently understates the gap. Confined to `java.lang`, whose
/// contents are fixed by the language rather than by a dependency: anything
/// reached through an `import` is classified by [`is_outside_repository`]
/// instead, on the evidence of the import itself rather than on a list.
///
/// Deliberately not exhaustive. A missing entry leaves a reference in the
/// unexplained half, which is the safe direction.
const JAVA_LANG_TYPES: &[&str] = &[
    // Core.
    "Object", "String", "StringBuilder", "StringBuffer", "CharSequence", "Comparable", "Iterable",
    "Runnable", "Thread", "Class", "Enum", "Record", "Number", "Boolean", "Byte", "Character",
    "Short", "Integer", "Long", "Float", "Double", "Void", "Math", "StrictMath", "System",
    "Runtime", "Process", "ProcessBuilder", "ClassLoader", "Package", "Module", "ThreadLocal",
    "StackTraceElement", "Cloneable", "AutoCloseable", "Appendable", "Readable",
    // Throwables.
    "Throwable", "Exception", "RuntimeException", "Error", "AssertionError",
    "IllegalArgumentException", "IllegalStateException", "NullPointerException",
    "IndexOutOfBoundsException", "ArrayIndexOutOfBoundsException",
    "StringIndexOutOfBoundsException", "ClassCastException", "NumberFormatException",
    "UnsupportedOperationException", "ArithmeticException", "InterruptedException",
    "CloneNotSupportedException", "ClassNotFoundException", "NoSuchMethodException",
    "NoSuchFieldException", "IllegalAccessException", "InstantiationException",
    "SecurityException", "OutOfMemoryError", "StackOverflowError", "NegativeArraySizeException",
    "ArrayStoreException", "NoClassDefFoundError", "ExceptionInInitializerError", "LinkageError",
    "VirtualMachineError", "IllegalMonitorStateException", "IllegalThreadStateException",
    // Annotations.
    "Override", "Deprecated", "SuppressWarnings", "SafeVarargs", "FunctionalInterface",
];

fn is_java_lang_type(simple: &str) -> bool {
    JAVA_LANG_TYPES.contains(&simple)
}

/// Whether a fully-qualified name lies outside the analysed tree.
///
/// Decided by package rather than by the name itself, and that distinction is
/// the whole of it. `index.by_fqn` missing an entry means only that nothing was
/// indexed under that name, which happens for a nested class: gson imports
/// `com.google.gson.common.TestTypes.BagOfPrimitives`, that exact string is not
/// a key, and the class is nevertheless in the repository. Classifying on the
/// missing key would report 235 of gson's own types as somebody else's code.
///
/// So a name counts as outside only when *no* dotted prefix of it is a package
/// some file in this tree declares. `java.io.IOException` qualifies;
/// `com.google.gson.common.TestTypes.BagOfPrimitives` does not, because
/// `com.google.gson.common` is declared here — which makes it a reference this
/// resolver missed, and it stays in the unexplained half where it belongs.
fn is_outside_repository(fqn: &str, repo_packages: &BTreeSet<String>) -> bool {
    let parts: Vec<&str> = fqn.split('.').collect();
    for take in (1..=parts.len()).rev() {
        if repo_packages.contains(&parts[..take].join(".")) {
            return false;
        }
    }
    true
}

/// Whether an unresolved type reference can be shown to be outside the tree.
///
/// Two kinds of evidence, and nothing weaker. An explicit import names the
/// package the type comes from, so the import statement itself settles it. A
/// name with no import at all can still be `java.lang`, which needs none.
///
/// A name reachable only through a wildcard import is deliberately *not*
/// classified: `import java.util.*` alongside `import com.example.thing.*`
/// leaves no way to say which one a bare `Thing` came from, and a guess there
/// would be exactly the name-matching this project exists to avoid.
fn type_is_outside(simple: &str, facts: &FileFacts, repo_packages: &BTreeSet<String>) -> bool {
    if let Some((_, fqn)) = facts.imports.iter().find(|(name, _)| name == simple) {
        return is_outside_repository(fqn, repo_packages);
    }
    // No import: `java.lang` is in scope implicitly, and nothing else is.
    //
    // Safe despite wildcards, because of where this is reached from. Step 4 of
    // `resolve_type` has already tried every wildcard package against the
    // index and found nothing, so no package in this repository offers a type
    // by this name — a repo class shadowing a `java.lang` one would have bound
    // and never arrived here.
    is_java_lang_type(simple)
}

/// Whether an unresolved member reference can be shown to be outside the tree.
///
/// A static import names the type declaring the member, so it settles the
/// member the way an ordinary import settles a type — this is what places
/// gson's 1,100-odd `assertThat` calls, which come from Truth. Failing that,
/// a member reached through a receiver whose *type* is outside is outside too:
/// `Map.put` cannot be in this repository if `Map` is not.
fn member_is_outside(
    owner: &str,
    member: &str,
    facts: &FileFacts,
    repo_packages: &BTreeSet<String>,
) -> bool {
    if let Some((_, declaring)) = facts.static_imports.iter().find(|(name, _)| name == member) {
        return is_outside_repository(declaring, repo_packages);
    }
    type_is_outside(owner, facts, repo_packages)
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
