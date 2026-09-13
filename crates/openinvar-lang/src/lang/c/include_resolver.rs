//! Resolving `#include` directives and the type names they bring into scope.
//!
//! Two bugs lived here. Includes resolved to `local_header::<name>`, a string
//! that is neither a symbol id nor an `ext::` package, so
//! [`openinvar_core::graph::InvarGraph::add_relationship`] dropped every one of
//! them — a C repository produced a graph with zero edges. And type names were
//! looked up in a repository-wide, first-wins map, so a `typedef struct Foo Bar`
//! bound to whichever `Foo` happened to be indexed first regardless of what the
//! file could actually see.
//!
//! Includes now resolve to the header's module symbol, and a type name is only
//! visible if the file defines it or includes a header that does — which is
//! what the C compiler does too.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use openinvar_core::ir::{
    Diagnostic, DiagnosticCategory, DiagnosticLevel, RelationshipKind, RepoIR, SymbolKind,
};
use openinvar_core::symbol_id::{
    external_package_id, kind_suffix, module_symbol_id, parse_symbol_id,
    parse_unresolved_local_type_id,
};

const INCLUDE_PREFIX: &str = "unresolved_include";

/// Placeholder for an `#include`, remembering whether it was quoted or angled.
///
/// The distinction matters: `"foo.h"` is searched relative to the including
/// file and is almost always in-repo, while `<foo.h>` comes from an include
/// path we cannot see and is almost always a system or third-party header.
pub fn unresolved_include_id(path: &str, is_local: bool) -> String {
    let kind = if is_local { "local" } else { "system" };
    format!("{INCLUDE_PREFIX}|{kind}|{path}")
}

fn parse_unresolved_include_id(raw: &str) -> Option<(bool, &str)> {
    let rest = raw.strip_prefix(INCLUDE_PREFIX)?.strip_prefix('|')?;
    let (kind, path) = rest.split_once('|')?;
    Some((kind == "local", path))
}

const PROTOTYPE_PREFIX: &str = "unresolved_prototype";

/// Placeholder for a function *declaration* — `int point_distance(...);`.
///
/// A prototype is not a symbol. It names a function defined in another
/// translation unit, so minting a node for it would put two nodes in the graph
/// for one function and make the name ambiguous to `find_symbol_id` — the
/// ambiguity 0.2.0 spent a release removing. These placeholders exist only
/// between extraction and resolution, and [`resolve_repo_ir`] drops every one
/// of them before returning.
pub fn unresolved_prototype_id(name: &str) -> String {
    format!("{PROTOTYPE_PREFIX}|{name}")
}

fn parse_unresolved_prototype_id(raw: &str) -> Option<&str> {
    raw.strip_prefix(PROTOTYPE_PREFIX)?.strip_prefix('|')
}

/// Which definition each header's prototypes stand for.
///
/// C splits a call across two files: the caller includes a header that
/// *declares* the function, and the definition lives in a `.c` file the caller
/// never sees. A caller must attach to the **definition** — the question the
/// graph answers is "what breaks if I change this", and a caller attached to
/// the prototype would leave `blast-radius` on the definition returning
/// nothing, which is the exact failure call edges exist to prevent.
///
/// The link is made by agreement between two files rather than by matching a
/// name across the repository:
///
/// 1. header `H` declares `N`, and
/// 2. exactly one file both defines `N` and includes `H`.
///
/// Condition 2 is what keeps this from being the repo-wide leaf matching that
/// 0.2.0 removed. It is anchored on a specific header the two files share, so
/// two unrelated projects in one repository never cross, and a `static` helper
/// that happens to share a name with someone else's function makes the match
/// ambiguous rather than wrong. Including the header that declares you is also
/// exactly what a C build does to have the compiler check the two agree, so
/// the rule keys on a fact the language already enforces rather than on a
/// filename convention like `geometry.c` implementing `geometry.h`.
///
/// Where no unique definer exists the prototype links to nothing and the call
/// records no edge, as before.
fn link_prototypes(
    repo_ir: &RepoIR,
    includes: &HashMap<String, Vec<String>>,
) -> BTreeMap<(String, String), String> {
    // header → the names it declares.
    let mut declared: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for file in &repo_ir.files {
        for rel in &file.relationships {
            if let Some(name) = parse_unresolved_prototype_id(&rel.to) {
                declared
                    .entry(file.file.clone())
                    .or_default()
                    .insert(name.to_string());
            }
        }
    }

    // name → the files that define it, and the symbol id of each definition.
    let mut defined: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for file in &repo_ir.files {
        for symbol in &file.symbols {
            if symbol.kind == SymbolKind::Function {
                defined
                    .entry(symbol.name.clone())
                    .or_default()
                    .push((file.file.clone(), symbol.id.clone()));
            }
        }
    }

    let mut links = BTreeMap::new();
    for (header, names) in &declared {
        for name in names {
            let Some(candidates) = defined.get(name) else {
                continue;
            };
            let mut definers = candidates.iter().filter(|(definer, _)| {
                includes
                    .get(definer)
                    .is_some_and(|headers| headers.contains(header))
            });

            let Some((_, id)) = definers.next() else {
                continue;
            };
            // Ambiguous: more than one file defines this name and includes this
            // header. Record nothing rather than pick one.
            if definers.next().is_some() {
                continue;
            }
            links.insert((header.clone(), name.clone()), id.clone());
        }
    }

    links
}

pub fn resolve_repo_ir(_root: &std::path::Path, repo_ir: &mut RepoIR) {
    let known_files: HashSet<String> = repo_ir.files.iter().map(|f| f.file.clone()).collect();

    // Basename → files with that name, for includes that rely on an `-I` path
    // we cannot reconstruct. Only used when the name is unambiguous.
    let mut by_basename: HashMap<String, Vec<String>> = HashMap::new();
    for file in &repo_ir.files {
        by_basename
            .entry(basename(&file.file).to_string())
            .or_default()
            .push(file.file.clone());
    }

    // Symbols each file defines, for building the visible set below.
    let symbols_by_file: HashMap<String, HashMap<String, String>> = repo_ir
        .files
        .iter()
        .map(|file| {
            let symbols = file
                .symbols
                .iter()
                .filter(|s| s.kind != SymbolKind::Module)
                .map(|s| (s.name.clone(), s.id.clone()))
                .collect();
            (file.file.clone(), symbols)
        })
        .collect();

    // ── pass 1: includes ─────────────────────────────────────
    //
    // Done for every file up front, because pass 2 needs the include graph to
    // know which types each file can see.
    let mut includes: HashMap<String, Vec<String>> = HashMap::new();

    for file in &mut repo_ir.files {
        let path = file.file.clone();
        let mut resolved_includes = Vec::new();

        for rel in file.relationships.iter_mut() {
            let Some((is_local, include_path)) = parse_unresolved_include_id(&rel.to) else {
                continue;
            };
            let include_path = include_path.to_string();

            let target = if is_local {
                resolve_relative(&path, &include_path)
                    .filter(|candidate| known_files.contains(candidate))
                    .or_else(|| unique_basename_match(&by_basename, &include_path))
            } else {
                // An angled include may still be in-repo when the build passes
                // `-I`; accept it only on an unambiguous basename match.
                unique_basename_match(&by_basename, &include_path)
            };

            match target {
                Some(target) => {
                    rel.to = module_symbol_id(&target);
                    resolved_includes.push(target);
                }
                None => {
                    // Not in the repository: a system or third-party header.
                    // Recorded as an external package so the dependency is
                    // visible rather than dropped.
                    rel.to = external_package_id(&package_name_for(&include_path));
                }
            }
        }

        includes.insert(path, resolved_includes);
    }

    // Which definition each header's prototypes stand for. Built once, after
    // the include graph exists and before any call is resolved against it.
    let prototypes = link_prototypes(repo_ir, &includes);

    // ── pass 2: type names ───────────────────────────────────
    for index in 0..repo_ir.files.len() {
        let path = repo_ir.files[index].file.clone();

        // Names this file reaches through a prototype in a header it includes,
        // transitively — a header may include the header that declares the
        // function.
        let mut via_prototype: HashMap<String, String> = HashMap::new();
        for header in reachable_headers(&path, &includes) {
            for ((declaring, name), id) in &prototypes {
                if *declaring == header {
                    via_prototype.insert(name.clone(), id.clone());
                }
            }
        }

        // What this translation unit can see: its own symbols plus those of the
        // headers it includes, transitively — headers include headers.
        let visible = visible_symbols(&path, &includes, &symbols_by_file);

        // Aliases introduced by `typedef` / `using`, resolved first so later
        // references through the alias find the underlying type.
        let mut aliases: HashMap<String, String> = HashMap::new();
        let mut resolved_props: HashMap<String, BTreeSet<String>> = HashMap::new();
        let mut drop = BTreeSet::new();
        let mut diagnostics = Vec::new();

        // Aliases must be resolved before the accesses that use them, and the
        // extractor emits them in source order, so two sweeps are needed.
        for rel in &repo_ir.files[index].relationships.clone() {
            if rel.kind != RelationshipKind::Imports {
                continue;
            }
            let (Some(type_name), Some(alias)) = (
                parse_unresolved_local_type_id(&rel.to),
                rel.alias.as_deref(),
            ) else {
                continue;
            };
            if let Some(id) = visible.get(type_name) {
                aliases.insert(alias.to_string(), id.clone());
            }
        }

        let file = &mut repo_ir.files[index];
        for (position, rel) in file.relationships.iter_mut().enumerate() {
            let Some(type_name) = parse_unresolved_local_type_id(&rel.to).map(str::to_string)
            else {
                continue;
            };

            let resolved = visible
                .get(&type_name)
                .or_else(|| aliases.get(&type_name))
                .cloned();

            // Calls resolve against the same visible set, but fail quietly:
            // the standard library is not in the graph, and a warning per
            // `printf` would bury the resolution warnings that matter.
            if rel.kind == RelationshipKind::Calls || rel.kind == RelationshipKind::Instantiates {
                // A call the visible set cannot place may still cross a
                // translation unit through a prototype. Tried second, so a
                // definition the caller can actually see always wins — a
                // `static` helper shadows the external function of the same
                // name, exactly as it does at compile time.
                let resolved = resolved.or_else(|| via_prototype.get(&type_name).cloned());
                match resolved {
                    // A functional cast in C++ — `Celsius(x)` — is spelled like
                    // a call and calls nothing. The resolved target's kind is
                    // what keeps it out of `--kind calls`.
                    Some(id) if kind_suffix_of(&id) == Some(kind_suffix(&SymbolKind::Class)) => {
                        rel.kind = RelationshipKind::Instantiates;
                        rel.context = "construction or cast".to_string();
                        rel.to = id;
                    }
                    Some(id) => rel.to = id,
                    None => {
                        drop.insert(position);
                    }
                }
                continue;
            }

            match resolved {
                Some(id) => {
                    if rel.kind == RelationshipKind::AccessesProperty {
                        resolved_props
                            .entry(id.clone())
                            .or_default()
                            .extend(rel.properties_accessed.iter().cloned());
                    }
                    rel.to = id;
                }
                None => {
                    drop.insert(position);
                    if !crate::lang::c::scope_analyzer::is_builtin_type(&type_name) {
                        diagnostics.push(Diagnostic {
                            level: DiagnosticLevel::Warning,
                            category: DiagnosticCategory::Resolution,
                            message: format!(
                                "unable to resolve type '{type_name}'; \
                                 no included header defines it"
                            ),
                            file: Some(path.clone()),
                            line: Some(rel.line),
                        });
                    }
                }
            }
        }

        // An alias import carries the members reached through the alias.
        for rel in file.relationships.iter_mut() {
            if rel.kind != RelationshipKind::Imports {
                continue;
            }
            if let Some(props) = resolved_props.get(&rel.to) {
                rel.properties_accessed = props.iter().cloned().collect();
            }
        }

        file.diagnostics.extend(diagnostics);

        if !drop.is_empty() {
            let mut position = 0usize;
            file.relationships.retain(|_| {
                let keep = !drop.contains(&position);
                position += 1;
                keep
            });
        }

        // Prototypes are resolution scaffolding, never graph content: a
        // declaration is not a symbol, and leaving one behind would put a
        // second node in the graph for a function that already has one.
        file.relationships
            .retain(|rel| parse_unresolved_prototype_id(&rel.to).is_none());
    }
}

/// Every in-repo header `file` can see, following includes transitively.
///
/// Shares the traversal shape of [`visible_symbols`] rather than its result,
/// because a prototype link is keyed by the header that declares the name, not
/// by the symbols that header defines.
fn reachable_headers(file: &str, includes: &HashMap<String, Vec<String>>) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut queue = vec![file.to_string()];

    while let Some(current) = queue.pop() {
        for header in includes.get(&current).into_iter().flatten() {
            if seen.insert(header.clone()) {
                queue.push(header.clone());
            }
        }
    }

    seen
}

/// The kind component of a resolved symbol id, if it is one.
///
/// Compared against [`kind_suffix`] rather than a string literal, so a rename
/// of the suffix in `symbol_id` moves both sides together.
fn kind_suffix_of(id: &str) -> Option<&str> {
    parse_symbol_id(id).map(|(_, _, kind)| kind)
}

/// Symbols reachable from `file`: its own, plus every header it includes.
fn visible_symbols(
    file: &str,
    includes: &HashMap<String, Vec<String>>,
    symbols_by_file: &HashMap<String, HashMap<String, String>>,
) -> HashMap<String, String> {
    let mut visible = HashMap::new();
    let mut seen = HashSet::new();
    let mut queue = vec![file.to_string()];

    while let Some(current) = queue.pop() {
        if !seen.insert(current.clone()) {
            continue; // headers include each other; visit each once
        }
        if let Some(symbols) = symbols_by_file.get(&current) {
            for (name, id) in symbols {
                // The including file wins on a clash, matching C's shadowing.
                visible.entry(name.clone()).or_insert_with(|| id.clone());
            }
        }
        if let Some(headers) = includes.get(&current) {
            queue.extend(headers.iter().cloned());
        }
    }

    visible
}

/// Resolve `#include "../include/foo.h"` against the including file's directory.
fn resolve_relative(from_file: &str, include_path: &str) -> Option<String> {
    let mut parts: Vec<&str> = from_file.split('/').collect();
    parts.pop(); // drop the filename

    for segment in include_path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }

    Some(parts.join("/"))
}

/// Match on filename alone, but only when exactly one file could be meant.
fn unique_basename_match(
    by_basename: &HashMap<String, Vec<String>>,
    include_path: &str,
) -> Option<String> {
    let candidates = by_basename.get(basename(include_path))?;
    match candidates.as_slice() {
        [only] => Some(only.clone()),
        _ => None,
    }
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// A stable node name for a header outside the repository.
///
/// `<sys/socket.h>` becomes `sys/socket.h`, keeping system headers from the
/// same family grouped rather than collapsed into one opaque node.
fn package_name_for(include_path: &str) -> String {
    include_path.trim_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_includes_normalise_against_the_including_file() {
        assert_eq!(
            resolve_relative("src/view_model_mapper.c", "../include/user_payload.h").as_deref(),
            Some("include/user_payload.h")
        );
        assert_eq!(
            resolve_relative("src/a/b.c", "./local.h").as_deref(),
            Some("src/a/local.h")
        );
        assert_eq!(
            resolve_relative("main.c", "lib/util.h").as_deref(),
            Some("lib/util.h")
        );
    }

    #[test]
    fn includes_escaping_the_root_do_not_resolve() {
        assert_eq!(resolve_relative("main.c", "../../outside.h"), None);
    }

    #[test]
    fn include_placeholders_round_trip() {
        let local = unresolved_include_id("../include/a.h", true);
        assert_eq!(
            parse_unresolved_include_id(&local),
            Some((true, "../include/a.h"))
        );
        let system = unresolved_include_id("stdio.h", false);
        assert_eq!(
            parse_unresolved_include_id(&system),
            Some((false, "stdio.h"))
        );
    }
}
