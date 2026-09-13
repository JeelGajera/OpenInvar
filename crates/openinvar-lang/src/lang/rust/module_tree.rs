//! Mapping Rust module paths to the files and symbols they name.
//!
//! Resolution here is path-directed: a name is only found in the module that
//! actually declares or re-exports it. An earlier implementation resolved a
//! `use` path by looking up its final segment in a repository-wide map of
//! every symbol name, which made `use std::fmt::Debug` resolve to a local
//! `Debug` if one existed anywhere.
//!
//! Module paths are absolute and namespaced by the crate that owns them —
//! `crates/openinvar-core/src/ir.rs` is `openinvar_core::ir`. See
//! [`crate::lang::rust::crate_set`] for why, and for what that fixed: the previous version
//! assumed a single crate rooted at `<repo>/src/`, so nothing in a Cargo
//! workspace resolved at all.

use std::collections::HashMap;
use std::path::Path;

use openinvar_core::ir::{FileIR, RelationshipKind, SymbolKind};
use openinvar_core::symbol_id::{module_symbol_id, parse_unresolved_import_id};

use crate::lang::rust::crate_set::CrateSet;

/// How far a `pub use` chain is followed before giving up.
///
/// Real re-export chains are two or three deep (`lib.rs` → `mod.rs` → the
/// defining module); the bound stops a cyclic `pub use` from looping forever.
const MAX_REEXPORT_DEPTH: usize = 8;

/// Path prefixes that are always external, whatever the tree contains.
///
/// A crate in the repository could in principle be named `std`, but resolving
/// `use std::fmt` to it would be far more wrong than the reverse.
const ALWAYS_EXTERNAL: [&str; 4] = ["std", "core", "alloc", "proc_macro"];

/// What a `use` path pointed at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// A symbol defined in this repository.
    Symbol(String),
    /// A module in this repository, imported whole (`use foo::*`, `use foo;`).
    Module(String),
    /// A third-party crate, named by its first path segment.
    External(String),
    /// A local module path that exists, but does not export this name.
    UnknownMember,
    /// Nothing in the repository matches, and it does not look external either.
    Unknown,
}

#[derive(Debug, Default)]
pub struct ModuleTree {
    /// `openinvar_core::ir` → `crates/openinvar-core/src/ir.rs`
    module_to_file: HashMap<String, String>,
    /// file → symbol name → symbol id
    symbols_by_file: HashMap<String, HashMap<String, String>>,
    /// file → local name → the `(module, symbol)` it was imported from.
    /// Used to follow `pub use` re-export chains.
    imports_by_file: HashMap<String, HashMap<String, (String, String)>>,
    /// The packages and crate roots discovered in the tree.
    crates: CrateSet,
}

impl ModuleTree {
    pub fn build(root: &Path, files: &[FileIR]) -> Self {
        let paths: Vec<String> = files.iter().map(|f| f.file.clone()).collect();
        let crates = CrateSet::discover(root, &paths);

        let mut tree = Self {
            crates,
            ..Default::default()
        };

        for file in files {
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

            if let Some(module) = tree.crates.module_path_for(&file.file) {
                tree.module_to_file.insert(module, file.file.clone());
            }
            tree.symbols_by_file.insert(file.file.clone(), symbols);
            tree.imports_by_file.insert(file.file.clone(), imports);
        }

        tree
    }

    /// Resolve `module::symbol` as written inside `current_file`.
    pub fn resolve(&self, current_file: &str, module: &str, symbol: &str) -> Resolved {
        let absolute = self.absolutize(current_file, module);

        let Some(target_file) = self.module_to_file.get(&absolute) else {
            return if self.crates.is_local_namespace(&absolute) {
                Resolved::Unknown
            } else {
                Resolved::External(first_segment(&absolute).to_string())
            };
        };

        if symbol == openinvar_core::symbol_id::IMPORT_ALL {
            return Resolved::Module(module_symbol_id(target_file));
        }

        match self.lookup_in_file(target_file, symbol, 0) {
            Some(id) => Resolved::Symbol(id),
            None => Resolved::UnknownMember,
        }
    }

    /// Find `symbol` in `file`, following `pub use` re-exports outward.
    fn lookup_in_file(&self, file: &str, symbol: &str, depth: usize) -> Option<String> {
        if let Some(id) = self.symbols_by_file.get(file).and_then(|s| s.get(symbol)) {
            return Some(id.clone());
        }
        if depth >= MAX_REEXPORT_DEPTH {
            return None;
        }

        // The module does not define it — it may re-export it.
        let (module, inner) = self.imports_by_file.get(file)?.get(symbol)?;
        let absolute = self.absolutize(file, module);
        let target = self.module_to_file.get(&absolute)?;
        if target == file {
            return None; // self-referential `pub use`, stop rather than loop
        }
        self.lookup_in_file(target, inner, depth + 1)
    }

    /// Expand a `use` path into an absolute, crate-namespaced module path.
    ///
    /// The namespace of a path written inside a crate is that crate's own, so
    /// `crate::ir` inside `openinvar-core` and `openinvar_core::ir` written
    /// anywhere else both land on `openinvar_core::ir`. Getting these to agree
    /// is what makes inter-crate resolution work at all.
    pub fn absolutize(&self, current_file: &str, module: &str) -> String {
        let here = self
            .crates
            .module_path_for(current_file)
            .unwrap_or_else(|| "crate".to_string());
        let namespace = here.split("::").next().unwrap_or(&here).to_string();

        // `::foo` names a crate explicitly rather than a local module.
        let module = module.strip_prefix("::").unwrap_or(module);

        if let Some(rest) = module.strip_prefix("crate::") {
            return join_module(&namespace, rest);
        }
        if module == "crate" {
            return namespace;
        }
        if let Some(rest) = module.strip_prefix("self::") {
            return join_module(&here, rest);
        }
        if module == "self" {
            return here;
        }
        if module.starts_with("super") {
            let mut base = here;
            let mut rest = module;
            while let Some(tail) = rest.strip_prefix("super") {
                base = parent_module(&base, &namespace);
                rest = tail.strip_prefix("::").unwrap_or("");
                if rest.is_empty() {
                    return base;
                }
                if !rest.starts_with("super") {
                    return join_module(&base, rest);
                }
            }
            return base;
        }

        let root = first_segment(module);

        if ALWAYS_EXTERNAL.contains(&root) {
            return module.to_string();
        }

        // A crate in this tree, possibly reached under a dependency rename.
        if let Some(target) = self.crates.extern_namespace(current_file, root) {
            // `use openinvar_core::ir::RepoIR` from anywhere, and `use foo::X`
            // where the manifest says `foo = { package = "bar" }`.
            let rest = module.strip_prefix(root).unwrap_or("");
            let rest = rest.strip_prefix("::").unwrap_or("");
            return join_module(&target, rest);
        }

        // A bare path is otherwise ambiguous. `use user::UserPayload;` inside
        // `models/mod.rs` means the sibling module `models::user` (always in
        // edition 2015, and still widely written), while `use serde::Serialize`
        // names an external crate. Prefer a sibling that actually exists.
        let sibling = join_module(&here, root);
        if self.module_to_file.contains_key(&sibling) {
            return join_module(&here, module);
        }

        // Then a module directly under this crate's root.
        let at_crate_root = join_module(&namespace, root);
        if self.module_to_file.contains_key(&at_crate_root) {
            return join_module(&namespace, module);
        }

        module.to_string()
    }
}

fn join_module(base: &str, rest: &str) -> String {
    if rest.is_empty() {
        base.to_string()
    } else if base.is_empty() {
        rest.to_string()
    } else {
        format!("{base}::{rest}")
    }
}

/// The parent of `module`, stopping at the crate namespace.
///
/// `super` at the crate root has nowhere to climb to, so it stays put rather
/// than walking off the top into another crate's namespace.
fn parent_module(module: &str, namespace: &str) -> String {
    match module.rfind("::") {
        Some(cut) => module[..cut].to_string(),
        None => namespace.to_string(),
    }
}

fn first_segment(path: &str) -> &str {
    path.split("::").next().unwrap_or(path)
}
