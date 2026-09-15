//! Turning placeholder ids into graph-addressable ones.
//!
//! Two passes over each file, in this order because the second depends on the
//! first: resolve imports to build the file's local-name table, then resolve
//! type references against that table.
//!
//! Anything still unresolved at the end is removed and reported. That matters
//! because [`openinvar_core::graph::InvarGraph::add_relationship`] silently
//! drops edges pointing at ids it does not know — so an unresolved placeholder
//! that reaches the graph is a missing edge nobody hears about. Reporting it
//! here turns a silent gap into a diagnostic the user can act on.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use openinvar_core::ir::{
    Diagnostic, DiagnosticCategory, DiagnosticLevel, RelationshipKind, RepoIR, SymbolKind,
};
use openinvar_core::symbol_id::{
    external_package_id, is_external_package, kind_suffix, parse_symbol_id,
    parse_unresolved_import_id, parse_unresolved_local_type_id,
};

use crate::lang::rust::module_tree::{ModuleTree, Resolved};
use crate::lang::rust::scope_analyzer::is_builtin_type;

pub fn resolve_repo_ir(root: &Path, repo_ir: &mut RepoIR) {
    let tree = ModuleTree::build(root, &repo_ir.files);

    // Symbols of every file, keyed by the file that defines them. Needed only
    // by `Foo::new()`, where the method lives in whichever file defines `Foo`
    // rather than in the file making the call. Lookups are always anchored to
    // a file already resolved from this file's imports — never a repository-
    // wide search for a leaf name, which is the bug 0.2.0 fixed here.
    //
    // Keyed by the *qualified* name from the symbol id rather than by
    // `Symbol::name`: a method's display name is the bare `new`, which two
    // types in one file both carry. The id's name component is `UserService::
    // new`, which is what a call site spells and what distinguishes them.
    let symbols_by_file: HashMap<String, HashMap<String, String>> = repo_ir
        .files
        .iter()
        .map(|f| {
            let symbols = f
                .symbols
                .iter()
                .filter(|s| s.kind != SymbolKind::Module)
                .filter_map(|s| {
                    let (_, qualified, _) = parse_symbol_id(&s.id)?;
                    Some((qualified.to_string(), s.id.clone()))
                })
                .collect();
            (f.file.clone(), symbols)
        })
        .collect();

    for file in &mut repo_ir.files {
        let path = file.file.clone();

        // Symbols this file defines, for resolving references to its own types.
        let own_symbols: HashMap<String, String> = file
            .symbols
            .iter()
            .filter(|s| s.kind != SymbolKind::Module)
            .map(|s| (s.name.clone(), s.id.clone()))
            .collect();

        // ── pass 1: imports ──────────────────────────────────
        //
        // Populates the table that pass 2 resolves type names against.
        let mut local_names: HashMap<String, String> = HashMap::new();
        // References this file named that nothing here could place. Counted at
        // the point of the decision; what gets emitted is unchanged.
        let mut unbound: u32 = 0;
        let mut drop_import = Vec::new();

        for (index, rel) in file.relationships.iter_mut().enumerate() {
            if rel.kind != RelationshipKind::Imports {
                continue;
            }
            let Some((module, symbol)) = parse_unresolved_import_id(&rel.to) else {
                continue;
            };
            let (module, symbol) = (module.to_string(), symbol.to_string());

            // The name this import is known by for the rest of the file.
            let local = rel.alias.clone().unwrap_or_else(|| {
                if symbol == openinvar_core::symbol_id::IMPORT_ALL {
                    last_segment(&module).to_string()
                } else {
                    symbol.clone()
                }
            });

            match tree.resolve(&path, &module, &symbol) {
                Resolved::Symbol(id) | Resolved::Module(id) => {
                    rel.to = id.clone();
                    local_names.insert(local, id);
                }
                Resolved::External(package) => {
                    let id = external_package_id(&package);
                    rel.to = id.clone();
                    local_names.insert(local, id);
                }
                Resolved::UnknownMember => {
                    // The module exists but does not export this name. Keep the
                    // file-level dependency, which is still true, and say so.
                    match tree.resolve(&path, &module, openinvar_core::symbol_id::IMPORT_ALL) {
                        Resolved::Module(module_id) => {
                            rel.to = module_id.clone();
                            local_names.insert(local, module_id);
                        }
                        _ => {
                            drop_import.push(index);
                            unbound += 1;
                        }
                    }
                    file.diagnostics.push(Diagnostic {
                        level: DiagnosticLevel::Warning,
                        category: DiagnosticCategory::Resolution,
                        message: format!(
                            "module '{module}' does not export '{symbol}'; \
                             recorded a dependency on the module instead"
                        ),
                        file: Some(path.clone()),
                        line: Some(rel.line),
                    });
                }
                Resolved::Unknown => {
                    drop_import.push(index);
                    unbound += 1;
                    file.diagnostics.push(Diagnostic {
                        level: DiagnosticLevel::Warning,
                        category: DiagnosticCategory::Resolution,
                        message: format!("unable to resolve import '{module}::{symbol}'"),
                        file: Some(path.clone()),
                        line: Some(rel.line),
                    });
                }
            }
        }

        // ── pass 2: type references ──────────────────────────
        //
        // `AccessesProperty` from field access, `Implements` from `impl … for`
        // and `#[derive(..)]`. Both carry the type name in the placeholder.
        let mut drop_type = Vec::new();
        let mut resolved_props: HashMap<String, BTreeSet<String>> = HashMap::new();

        for (index, rel) in file.relationships.iter_mut().enumerate() {
            let Some(type_name) = parse_unresolved_local_type_id(&rel.to).map(str::to_string)
            else {
                continue;
            };

            // Calls resolve differently from type references: a path callee
            // has to reach a method in another file, and an unresolved callee
            // is silence rather than a diagnostic.
            if rel.kind == RelationshipKind::Calls || rel.kind == RelationshipKind::Instantiates {
                match resolve_callee(&type_name, &local_names, &own_symbols, &symbols_by_file) {
                    // `Foo(1)` on a tuple struct, and `Enum::Variant(x)` on a
                    // tuple variant, are construction spelled as a call. The
                    // resolved target's kind settles it, the same way it does
                    // in Python — and it has to, because nothing at the call
                    // site distinguishes them from a function call. Leaving
                    // these as `Calls` would put an edge in the graph saying a
                    // variant was called, which nothing ever does.
                    Some(id) if constructs(&id) => {
                        rel.kind = RelationshipKind::Instantiates;
                        rel.context = "instantiation".to_string();
                        rel.to = id;
                    }
                    // A call into a crate from crates.io: the only id available
                    // is the package, and an edge saying a *package* was called
                    // would be counted by blast-radius as a dependent of
                    // something nobody can open. The import edge already
                    // records that dependency.
                    Some(id) if is_external_package(&id) => {
                        drop_type.push(index);
                    }
                    Some(id) => rel.to = id,
                    // A prelude name (`Some`, `drop`), a macro-generated call,
                    // or a fully-qualified path used without a `use`. None of
                    // them name a symbol in the graph, and no diagnostic is
                    // raised because there is nothing here a user could fix.
                    // Counted: this arm cannot tell a prelude name from a
                    // local function the resolver missed, and excluding what it
                    // cannot classify would measure confidence, not coverage.
                    None => {
                        drop_type.push(index);
                        unbound += 1;
                    }
                }
                continue;
            }

            let resolved = local_names
                .get(&type_name)
                .or_else(|| own_symbols.get(&type_name))
                .cloned();

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
                    drop_type.push(index);
                    // Standard-library types are genuine references to code
                    // outside the repository, not resolution failures — so they
                    // are not counted as unbound either.
                    if !is_builtin_type(&type_name) {
                        unbound += 1;
                    }
                    if !is_builtin_type(&type_name) {
                        file.diagnostics.push(Diagnostic {
                            level: DiagnosticLevel::Warning,
                            category: DiagnosticCategory::Resolution,
                            message: format!(
                                "unable to resolve type '{type_name}' referenced in {path}"
                            ),
                            file: Some(path.clone()),
                            line: Some(rel.line),
                        });
                    }
                }
            }
        }

        // ── pass 3: attribute properties to their import ─────
        //
        // Keyed by resolved target, so a file that touches two imported types
        // gives each one only its own fields.
        for rel in &mut file.relationships {
            if rel.kind != RelationshipKind::Imports {
                continue;
            }
            if let Some(props) = resolved_props.get(&rel.to) {
                rel.properties_accessed = props.iter().cloned().collect();
            }
        }

        file.unbound_references = unbound;

        let mut unresolved: BTreeSet<usize> = drop_import.into_iter().collect();
        unresolved.extend(drop_type);
        if !unresolved.is_empty() {
            let mut index = 0usize;
            file.relationships.retain(|_| {
                let keep = !unresolved.contains(&index);
                index += 1;
                keep
            });
        }
    }
}

/// Bind a callee name to the symbol that actually runs.
///
/// Three shapes, in order:
///
/// 1. `foo` — a free function or tuple struct, resolved through the file's
///    imports and then its own symbols, exactly like a type reference.
/// 2. `Foo::new` where `Foo` is defined in this file — the method symbol is
///    named `Foo::new`, so the file's own table already holds it.
/// 3. `Foo::new` where `Foo` was imported — resolve `Foo` first, then look the
///    method up **in the file that defines `Foo`**. The lookup is anchored to
///    that file rather than searched for repository-wide; matching a leaf name
///    across the repository is the bug 0.2.0 fixed in this adapter.
///
/// A path with more than one segment before the method (`crate::foo::Foo::new`
/// with no `use`) returns `None`. Resolving it would mean guessing which `Foo`
/// was meant, and "fully-qualified paths used inline without a `use` record no
/// edge" is a limit the README states and this keeps true.
fn resolve_callee(
    callee: &str,
    local_names: &HashMap<String, String>,
    own_symbols: &HashMap<String, String>,
    symbols_by_file: &HashMap<String, HashMap<String, String>>,
) -> Option<String> {
    if let Some(id) = local_names.get(callee).or_else(|| own_symbols.get(callee)) {
        return Some(id.clone());
    }

    let (type_path, method) = callee.rsplit_once("::")?;
    if type_path.contains("::") {
        return None;
    }

    let type_id = local_names
        .get(type_path)
        .or_else(|| own_symbols.get(type_path))?;

    // An associated function on a type from another crate is not a symbol we
    // hold; the import edge already carries that dependency.
    let (defining_file, _, _) = parse_symbol_id(type_id)?;
    symbols_by_file
        .get(defining_file)?
        .get(&format!("{type_path}::{method}"))
        .cloned()
}

/// Whether a resolved target is a thing that gets constructed rather than run.
///
/// A tuple struct and a tuple enum variant are both invoked with call syntax
/// and neither is a function. The distinction cannot be made at the call site,
/// only from the symbol the call resolves to.
fn constructs(id: &str) -> bool {
    matches!(
        kind_suffix_of(id),
        Some(suffix)
            if suffix == kind_suffix(&SymbolKind::Class)
                || suffix == kind_suffix(&SymbolKind::EnumVariant)
    )
}

/// The kind component of a resolved symbol id, if it is one.
///
/// Compared against [`kind_suffix`] rather than a string literal, so a rename
/// of the suffix in `symbol_id` moves both sides together.
fn kind_suffix_of(id: &str) -> Option<&str> {
    parse_symbol_id(id).map(|(_, _, kind)| kind)
}

fn last_segment(path: &str) -> &str {
    path.rsplit("::").next().unwrap_or(path)
}
