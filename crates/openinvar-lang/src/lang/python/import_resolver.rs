//! Resolving Python imports against the repository's module layout.
//!
//! The previous index held only classes and interfaces, so
//! `from .utils import compute_total` — a function, and the shape of most
//! Python imports — fell through to a fallback that labelled it
//! `ext::utils::package`. That did not merely fail to resolve: it asserted a
//! dependency on a third-party package named `utils` that does not exist, and
//! made the real function invisible to `blast-radius`.
//!
//! Every symbol kind is indexed here, and a module that resolves locally but
//! does not export the name produces a diagnostic rather than an invented
//! external package.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use openinvar_core::ir::{
    Diagnostic, DiagnosticCategory, DiagnosticLevel, RelationshipKind, RepoIR, SymbolKind,
};
use openinvar_core::symbol_id::{
    external_package_id, is_external_package, kind_suffix, module_symbol_id, parse_symbol_id,
    parse_unresolved_import_id, parse_unresolved_local_type_id, IMPORT_ALL,
};

use crate::lang::python::scope_analyzer::is_builtin_type;

/// How far `__init__.py` re-export chains are followed.
const MAX_REEXPORT_DEPTH: usize = 8;

pub fn resolve_repo_ir(_root: &Path, repo_ir: &mut RepoIR) {
    // ── index the module layout ──────────────────────────────
    let mut module_to_file: HashMap<String, String> = HashMap::new();
    let mut symbols_by_file: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut imports_by_file: HashMap<String, HashMap<String, (String, String)>> = HashMap::new();

    for file in &repo_ir.files {
        let module = file_to_module(&file.file);

        // Every kind, not just classes: functions, constants and variables are
        // imported at least as often as types.
        let symbols: HashMap<String, String> = file
            .symbols
            .iter()
            .filter(|s| s.kind != SymbolKind::Module)
            .map(|s| (s.name.clone(), s.id.clone()))
            .collect();

        let imports: HashMap<String, (String, String)> = file
            .relationships
            .iter()
            .filter(|r| r.kind == RelationshipKind::Imports)
            .filter_map(|r| {
                let (module, symbol) = parse_unresolved_import_id(&r.to)?;
                let local = r.alias.clone().unwrap_or_else(|| symbol.to_string());
                Some((local, (module.to_string(), symbol.to_string())))
            })
            .collect();

        module_to_file.insert(module, file.file.clone());
        symbols_by_file.insert(file.file.clone(), symbols);
        imports_by_file.insert(file.file.clone(), imports);
    }

    let index = Index {
        module_to_file,
        symbols_by_file,
        imports_by_file,
    };

    // ── resolve per file ─────────────────────────────────────
    for file in &mut repo_ir.files {
        let path = file.file.clone();
        let own_symbols = index
            .symbols_by_file
            .get(&path)
            .cloned()
            .unwrap_or_default();

        let mut local_names: HashMap<String, String> = HashMap::new();
        let mut drop = BTreeSet::new();
        // References this file named that nothing here could place. Counted at
        // the point of the decision; what gets emitted is unchanged.
        let mut unbound: u32 = 0;

        for (position, rel) in file.relationships.iter_mut().enumerate() {
            if rel.kind != RelationshipKind::Imports {
                continue;
            }
            let Some((module_spec, symbol)) = parse_unresolved_import_id(&rel.to) else {
                continue;
            };
            let (module_spec, symbol) = (module_spec.to_string(), symbol.to_string());
            let module = absolutize(&path, &module_spec);

            let local = rel.alias.clone().unwrap_or_else(|| {
                if symbol == IMPORT_ALL {
                    last_segment(&module).to_string()
                } else {
                    symbol.clone()
                }
            });

            // Importing a module rather than a name from it.
            if symbol == IMPORT_ALL {
                match index.module_to_file.get(&module) {
                    Some(target) => {
                        let id = module_symbol_id(target);
                        rel.to = id.clone();
                        local_names.insert(local, id);
                    }
                    None => {
                        let id = external_package_id(first_segment(&module));
                        rel.to = id.clone();
                        local_names.insert(local, id);
                    }
                }
                continue;
            }

            match index.resolve(&module, &symbol) {
                Resolution::Symbol(id) => {
                    rel.to = id.clone();
                    local_names.insert(local, id);
                }
                Resolution::MissingMember => {
                    // The module is ours, so this is a gap in our own analysis
                    // or a genuine broken import — never a third-party package.
                    let target = index
                        .module_to_file
                        .get(&module)
                        .map(|f| module_symbol_id(f));
                    match target {
                        Some(id) => {
                            rel.to = id.clone();
                            local_names.insert(local, id);
                        }
                        None => {
                            drop.insert(position);
                            unbound += 1;
                        }
                    }
                    file.diagnostics.push(Diagnostic {
                        level: DiagnosticLevel::Warning,
                        category: DiagnosticCategory::Resolution,
                        message: format!(
                            "local module '{module}' does not define '{symbol}'; \
                             recorded a dependency on the module instead"
                        ),
                        file: Some(path.clone()),
                        line: Some(rel.line),
                    });
                }
                Resolution::External => {
                    // `from fastapi import Depends` — a real third-party import.
                    let id = external_package_id(first_segment(&module));
                    rel.to = id.clone();
                    local_names.insert(local, id);
                }
            }
        }

        // Type references: attribute access and base classes.
        let mut resolved_props: HashMap<String, BTreeSet<String>> = HashMap::new();

        for (position, rel) in file.relationships.iter_mut().enumerate() {
            let Some(type_name) = parse_unresolved_local_type_id(&rel.to).map(str::to_string)
            else {
                continue;
            };

            let resolved = local_names
                .get(&type_name)
                .or_else(|| own_symbols.get(&type_name))
                .cloned()
                .or_else(|| resolve_dotted(&type_name, &local_names, &index));

            // Calls resolve against the same table, but neither fail the same
            // way nor keep the kind they were extracted with.
            if rel.kind == RelationshipKind::Calls {
                match resolved {
                    // `Foo()` is construction, `foo()` is invocation, and
                    // Python spells them identically. The resolved symbol
                    // settles it: promoting on the target's kind is a fact
                    // about what was called, where promoting on a capitalized
                    // name would only be a convention.
                    Some(id) if kind_suffix_of(&id) == Some(kind_suffix(&SymbolKind::Class)) => {
                        rel.kind = RelationshipKind::Instantiates;
                        rel.context = "instantiation".to_string();
                        rel.to = id;
                    }
                    // `from requests import get` then `get()`: the call is
                    // real, but the package is not the thing that ran. An edge
                    // saying a package was called would be counted as a
                    // dependent of the package by blast-radius, which is a
                    // claim about a symbol nobody can open. The Imports edge
                    // already records the dependency truthfully.
                    Some(id) if is_external_package(&id) => {
                        drop.insert(position);
                    }
                    Some(id) => rel.to = id,
                    // A builtin, a global, or a name from an untyped module
                    // binds to no symbol. No diagnostic: unlike an unresolved
                    // type, there is nothing here a user could fix, and
                    // `print()` in every file would drown the real warnings.
                    //
                    // Counted, though. This arm cannot tell `print()` from a
                    // local function the resolver missed, and a count that
                    // excluded what it cannot classify would be measuring its
                    // own confidence rather than its coverage.
                    None => {
                        drop.insert(position);
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
                    drop.insert(position);
                    // A builtin is positively identified as not a repository
                    // symbol, so it is not a reference anything failed to bind.
                    if !is_builtin_type(&type_name) {
                        unbound += 1;
                    }
                    if !is_builtin_type(&type_name) {
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

        for rel in file.relationships.iter_mut() {
            if rel.kind != RelationshipKind::Imports {
                continue;
            }
            if let Some(props) = resolved_props.get(&rel.to) {
                rel.properties_accessed = props.iter().cloned().collect();
            }
        }

        if !drop.is_empty() {
            let mut position = 0usize;
            file.relationships.retain(|_| {
                let keep = !drop.contains(&position);
                position += 1;
                keep
            });
        }
        file.unbound_references = unbound;
    }
}

/// Resolve `qualifier.Name`, where the qualifier is an imported module.
///
/// `class Order(models.Model)` after `from django.db import models` must reach
/// django; reducing the base to its final segment would look for a local
/// `Model` and find nothing.
fn resolve_dotted(
    type_name: &str,
    local_names: &HashMap<String, String>,
    index: &Index,
) -> Option<String> {
    let (qualifier, member) = type_name.split_once('.')?;
    let base = local_names.get(qualifier)?;

    // An external module: the member is external too, and the package node is
    // the most specific thing we can point at.
    if base.starts_with("ext::") {
        return Some(base.clone());
    }

    // A local module: look the member up inside it.
    let (module_file, _, _) = openinvar_core::symbol_id::parse_symbol_id(base)?;
    index.lookup(module_file, member, 0)
}

struct Index {
    module_to_file: HashMap<String, String>,
    symbols_by_file: HashMap<String, HashMap<String, String>>,
    imports_by_file: HashMap<String, HashMap<String, (String, String)>>,
}

enum Resolution {
    Symbol(String),
    /// The module is in this repository but does not export the name.
    MissingMember,
    /// The module is not in this repository at all.
    External,
}

impl Index {
    fn resolve(&self, module: &str, symbol: &str) -> Resolution {
        let Some(file) = self.module_to_file.get(module) else {
            return Resolution::External;
        };
        match self.lookup(file, symbol, 0) {
            Some(id) => Resolution::Symbol(id),
            None => Resolution::MissingMember,
        }
    }

    /// Find `symbol` in `file`, following `__init__.py` re-exports.
    ///
    /// A package's `__init__.py` typically imports names from its submodules so
    /// callers can write `from mypkg import Thing`; the definition lives one or
    /// more modules deeper.
    fn lookup(&self, file: &str, symbol: &str, depth: usize) -> Option<String> {
        if let Some(id) = self.symbols_by_file.get(file).and_then(|s| s.get(symbol)) {
            return Some(id.clone());
        }
        if depth >= MAX_REEXPORT_DEPTH {
            return None;
        }

        let (module, inner) = self.imports_by_file.get(file)?.get(symbol)?;
        let absolute = absolutize(file, module);
        let target = self.module_to_file.get(&absolute)?;
        if target == file {
            return None;
        }
        self.lookup(target, inner, depth + 1)
    }
}

/// Turn a file path into the dotted module path that names it.
fn file_to_module(file: &str) -> String {
    let path = file
        .trim_start_matches("./")
        .strip_suffix(".pyi")
        .or_else(|| file.trim_start_matches("./").strip_suffix(".py"))
        .unwrap_or(file);

    // `pkg/__init__.py` *is* the module `pkg`.
    let path = path.strip_suffix("/__init__").unwrap_or(path);
    path.replace('/', ".")
}

/// Expand a relative import (`.`, `..mod`) against the importing file.
///
/// One leading dot means the current package — the directory the file is in —
/// and each additional dot climbs one level.
fn absolutize(from_file: &str, module_spec: &str) -> String {
    if !module_spec.starts_with('.') {
        return module_spec.to_string();
    }

    let dots = module_spec.chars().take_while(|c| *c == '.').count();
    let suffix = module_spec[dots..].trim_start_matches('.');

    // Start from the importing file's package.
    let mut parts: Vec<&str> = from_file
        .trim_start_matches("./")
        .strip_suffix(".py")
        .unwrap_or(from_file)
        .split('/')
        .collect();
    parts.pop(); // the module itself
                 // `pkg/__init__.py` already names its package, so it does not climb again.
    if from_file.ends_with("/__init__.py") {
        // parts now points at the package directory's parent + package name
        // which is exactly the current package: nothing more to drop.
    }

    for _ in 0..dots.saturating_sub(1) {
        parts.pop();
    }

    let base = parts.join(".");
    match (base.is_empty(), suffix.is_empty()) {
        (true, _) => suffix.to_string(),
        (false, true) => base,
        (false, false) => format!("{base}.{suffix}"),
    }
}

fn first_segment(module: &str) -> &str {
    module.split('.').next().unwrap_or(module)
}

fn last_segment(module: &str) -> &str {
    module.rsplit('.').next().unwrap_or(module)
}

/// The kind component of a resolved symbol id, if it is one.
///
/// Compared against [`kind_suffix`] rather than a string literal, so a rename
/// of the suffix in `symbol_id` moves both sides together.
fn kind_suffix_of(id: &str) -> Option<&str> {
    parse_symbol_id(id).map(|(_, _, kind)| kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_paths_become_dotted_modules() {
        assert_eq!(file_to_module("models/user.py"), "models.user");
        assert_eq!(file_to_module("pkg/__init__.py"), "pkg");
        assert_eq!(file_to_module("app.py"), "app");
        assert_eq!(file_to_module("stubs/thing.pyi"), "stubs.thing");
    }

    #[test]
    fn relative_imports_climb_from_the_importing_package() {
        // One dot: the current package.
        assert_eq!(absolutize("app/main.py", ".helpers"), "app.helpers");
        // Two dots: the parent package.
        assert_eq!(absolutize("app/main.py", "..models"), "models");
        // Three dots from two levels deep.
        assert_eq!(
            absolutize("mappers/deep/view.py", "...models.user_payload"),
            "models.user_payload"
        );
    }

    #[test]
    fn absolute_imports_are_left_alone() {
        assert_eq!(absolutize("app/main.py", "os.path"), "os.path");
        assert_eq!(absolutize("app/main.py", "fastapi"), "fastapi");
    }
}
