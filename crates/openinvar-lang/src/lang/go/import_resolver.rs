//! Resolving Go imports and qualified references.
//!
//! Go imports a package, not a symbol, so an import edge should point at the
//! package. The previous implementation pointed it at `symbol_ids.first()` —
//! an arbitrary member decided by filename sort order, which meant adding an
//! unrelated file to a package silently repointed every edge into it.
//!
//! Here an import edge points at a synthetic package symbol, and the members a
//! file actually uses are recorded separately from the qualified references
//! (`models.UserPayload`) and field accesses the scope analyzer resolved.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use openinvar_core::ir::{
    Diagnostic, DiagnosticCategory, DiagnosticLevel, Language, RelationshipKind, RepoIR, Symbol,
    SymbolKind,
};
use openinvar_core::symbol_id::{
    external_package_id, kind_suffix, make_symbol_id, parse_symbol_id, parse_unresolved_import_id,
    parse_unresolved_local_type_id,
};

use crate::lang::go::module_resolver::ModuleSet;
use crate::lang::go::scope_analyzer::is_builtin_type;

/// A Go package: a directory's worth of files sharing a namespace.
#[derive(Debug, Default)]
struct Package {
    /// Synthetic symbol every import of this package points at.
    symbol_id: String,
    /// Package-scoped symbol names → ids. Go has no file-level visibility, so
    /// a name declared in any file of the package is visible from all of them.
    symbols: HashMap<String, String>,
    /// File that hosts the synthetic symbol, chosen deterministically.
    owner_file: String,
}

pub fn resolve_repo_ir(root: &Path, repo_ir: &mut RepoIR) {
    let file_paths: Vec<String> = repo_ir.files.iter().map(|f| f.file.clone()).collect();
    let modules = ModuleSet::discover(root, &file_paths);

    // ── index packages ───────────────────────────────────────
    let mut packages: BTreeMap<String, Package> = BTreeMap::new();
    let mut file_to_package: HashMap<String, String> = HashMap::new();

    for file in &repo_ir.files {
        let Some(import_path) = modules.import_path_for(&file.file) else {
            continue;
        };
        file_to_package.insert(file.file.clone(), import_path.clone());

        let package = packages.entry(import_path.clone()).or_insert_with(|| {
            let dir = directory_of(&file.file);
            Package {
                symbol_id: make_symbol_id(&dir, last_segment(&import_path), &SymbolKind::Module),
                symbols: HashMap::new(),
                owner_file: file.file.clone(),
            }
        });

        // Files arrive sorted, but pin the owner explicitly so the synthetic
        // symbol lands in the same file on every run regardless of input order.
        if file.file < package.owner_file {
            package.owner_file = file.file.clone();
        }
        for symbol in &file.symbols {
            if symbol.kind == SymbolKind::Module {
                continue;
            }
            package
                .symbols
                .entry(symbol.name.clone())
                .or_insert_with(|| symbol.id.clone());
        }
    }

    // ── materialise package symbols ──────────────────────────
    for (import_path, package) in &packages {
        let Some(owner) = repo_ir
            .files
            .iter_mut()
            .find(|f| f.file == package.owner_file)
        else {
            continue;
        };
        let dir = directory_of(&package.owner_file);
        owner.symbols.push(Symbol {
            id: package.symbol_id.clone(),
            name: last_segment(import_path).to_string(),
            kind: SymbolKind::Module,
            language: Language::Go,
            file: dir,
            line_start: 1,
            line_end: 1,
            signature: Some(format!("package {import_path}")),
        });
    }

    // ── resolve per file ─────────────────────────────────────
    for file in &mut repo_ir.files {
        let path = file.file.clone();
        let own_package = file_to_package.get(&path).cloned();

        // alias (or default package name) → import path
        let mut alias_to_package: HashMap<String, String> = HashMap::new();
        let mut drop = BTreeSet::new();
        // References this file named that nothing here could place. Counted at
        // the point of the decision; what gets emitted is unchanged.
        let mut unbound: u32 = 0;

        for rel in file.relationships.iter_mut() {
            if rel.kind != RelationshipKind::Imports {
                continue;
            }
            let Some((import_path, _)) = parse_unresolved_import_id(&rel.to) else {
                continue;
            };
            let import_path = import_path.to_string();
            let local = rel
                .alias
                .clone()
                .unwrap_or_else(|| last_segment(&import_path).to_string());

            match packages.get(&import_path) {
                Some(package) => {
                    rel.to = package.symbol_id.clone();
                    alias_to_package.insert(local, import_path);
                }
                None => {
                    // Inside a discovered module but with no matching package:
                    // the directory holds no Go we indexed. Everything else is
                    // a third-party dependency.
                    rel.to = external_package_id(last_segment(&import_path));
                    if modules.is_local_import(&import_path) {
                        file.diagnostics.push(Diagnostic {
                            level: DiagnosticLevel::Warning,
                            category: DiagnosticCategory::Resolution,
                            message: format!(
                                "import '{import_path}' is inside this module but no \
                                 package was indexed there"
                            ),
                            file: Some(path.clone()),
                            line: Some(rel.line),
                        });
                    }
                }
            }
        }

        // Type references: `models.UserPayload` (qualified) or `Foo` (same package).
        let mut resolved_props: HashMap<String, BTreeSet<String>> = HashMap::new();

        for (index, rel) in file.relationships.iter_mut().enumerate() {
            let Some(type_name) = parse_unresolved_local_type_id(&rel.to).map(str::to_string)
            else {
                continue;
            };

            let resolved = match type_name.split_once('.') {
                Some((alias, name)) => alias_to_package
                    .get(alias)
                    .and_then(|import_path| packages.get(import_path))
                    .and_then(|package| package.symbols.get(name))
                    .cloned(),
                None => own_package
                    .as_ref()
                    .and_then(|p| packages.get(p))
                    .and_then(|package| package.symbols.get(&type_name))
                    .cloned(),
            };

            // Calls resolve against the same table but fail quietly, and the
            // kind is settled by what the name turned out to be.
            if rel.kind == RelationshipKind::Calls || rel.kind == RelationshipKind::Instantiates {
                match resolved {
                    // `Foo(x)` on a type is a conversion, not a call — Go
                    // spells `int64(n)` and `New(n)` identically. The resolved
                    // target's kind is the only thing that distinguishes them,
                    // and an edge saying a type was called would surface under
                    // `--kind calls` as a caller that does not exist.
                    Some(id) if kind_suffix_of(&id) == Some(kind_suffix(&SymbolKind::Class)) => {
                        rel.kind = RelationshipKind::Instantiates;
                        rel.context = "conversion or literal".to_string();
                        rel.to = id;
                    }
                    Some(id) => rel.to = id,
                    // `make`, `len`, `append`, `panic`, or a name from a
                    // third-party package: none of them is a symbol in this
                    // graph. No diagnostic, because there is nothing a user
                    // could fix, and a warning per `len()` would bury the
                    // resolution warnings that matter.
                    // Counted: this arm cannot tell `len()` from a local
                    // function the resolver missed, and excluding what it
                    // cannot classify would measure confidence, not coverage.
                    None => {
                        drop.insert(index);
                        unbound += 1;
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
                    drop.insert(index);
                    // A qualified name whose package is a third-party import is
                    // an ordinary external reference, not a failure — and so is
                    // not counted as unbound.
                    let is_external_reference = type_name
                        .split_once('.')
                        .map(|(alias, _)| !alias_to_package.contains_key(alias))
                        .unwrap_or(false);
                    if !is_external_reference && !is_builtin_type(&type_name) {
                        unbound += 1;
                    }
                    if !is_external_reference && !is_builtin_type(&type_name) {
                        file.diagnostics.push(Diagnostic {
                            level: DiagnosticLevel::Warning,
                            category: DiagnosticCategory::Resolution,
                            message: format!("unable to resolve type '{type_name}'"),
                            file: Some(path.clone()),
                            line: Some(rel.line),
                        });
                    }
                }
            }
        }

        for rel in &mut file.relationships {
            if rel.kind != RelationshipKind::Imports {
                continue;
            }
            // An import edge carries the members used from that package, which
            // for a package node means the union across its used types.
            let props: BTreeSet<String> = resolved_props
                .iter()
                .filter(|(id, _)| {
                    packages
                        .values()
                        .any(|p| p.symbol_id == rel.to && p.symbols.values().any(|s| s == *id))
                })
                .flat_map(|(_, props)| props.iter().cloned())
                .collect();
            if !props.is_empty() {
                rel.properties_accessed = props.into_iter().collect();
            }
        }

        file.unbound_references = unbound;

        if !drop.is_empty() {
            let mut index = 0usize;
            file.relationships.retain(|_| {
                let keep = !drop.contains(&index);
                index += 1;
                keep
            });
        }
    }
}

/// The kind component of a resolved symbol id, if it is one.
///
/// Compared against [`kind_suffix`] rather than a string literal, so a rename
/// of the suffix in `symbol_id` moves both sides together.
fn kind_suffix_of(id: &str) -> Option<&str> {
    parse_symbol_id(id).map(|(_, _, kind)| kind)
}

fn directory_of(file: &str) -> String {
    match file.rfind('/') {
        Some(cut) => file[..cut].to_string(),
        None => ".".to_string(),
    }
}

fn last_segment(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
