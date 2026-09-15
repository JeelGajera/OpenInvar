use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use openinvar_core::ir::{
    Diagnostic, DiagnosticCategory, DiagnosticLevel, ReExportEntry, Relationship, RelationshipKind,
    RepoIR, Symbol, SymbolId,
};

use crate::lang::typescript::extractor::{
    is_builtin_type, parse_unresolved_import_symbol_id, parse_unresolved_local_type_symbol_id,
};
use crate::lang::typescript::tsconfig::TsConfigPaths;

// ── module specifier classification ──────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleKind {
    /// Starts with './' or '../' — must resolve to a local file
    Relative,
    /// npm package: 'react', '@org/package', 'jspdf'
    ExternalPackage,
    /// Node.js built-in: 'path', 'fs', 'os', etc.
    NodeBuiltin,
    /// Starts with '/' — absolute path (rare)
    Absolute,
}

const NODE_BUILTINS: &[&str] = &[
    "assert",
    "async_hooks",
    "buffer",
    "child_process",
    "cluster",
    "console",
    "constants",
    "crypto",
    "dgram",
    "diagnostics_channel",
    "dns",
    "domain",
    "events",
    "fs",
    "http",
    "http2",
    "https",
    "inspector",
    "module",
    "net",
    "os",
    "path",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "repl",
    "stream",
    "string_decoder",
    "timers",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "v8",
    "vm",
    "wasi",
    "worker_threads",
    "zlib",
];

pub fn classify_module_specifier(specifier: &str) -> ModuleKind {
    if specifier.starts_with("./") || specifier.starts_with("../") {
        return ModuleKind::Relative;
    }
    if specifier.starts_with('/') {
        return ModuleKind::Absolute;
    }
    // Handle 'node:fs' style imports
    let bare = specifier.strip_prefix("node:").unwrap_or(specifier);
    // Extract the package name (before any '/' subpath)
    let package_root = bare.split('/').next().unwrap_or(bare);
    if NODE_BUILTINS.contains(&package_root) {
        return ModuleKind::NodeBuiltin;
    }
    ModuleKind::ExternalPackage
}

/// Extract the package name from a module specifier.
/// `"react"` → `"react"`, `"react/jsx-runtime"` → `"react"`,
/// `"@org/pkg/sub"` → `"@org/pkg"`, `"node:fs"` → `"fs"`
fn extract_package_name(specifier: &str) -> String {
    let bare = specifier.strip_prefix("node:").unwrap_or(specifier);
    if bare.starts_with('@') {
        // Scoped: @org/pkg or @org/pkg/subpath
        let mut parts = bare.splitn(3, '/');
        let scope = parts.next().unwrap_or(bare);
        match parts.next() {
            Some(name) => format!("{scope}/{name}"),
            None => scope.to_string(),
        }
    } else {
        // Unscoped: pkg or pkg/subpath
        bare.split('/').next().unwrap_or(bare).to_string()
    }
}

pub fn resolve_repo_ir(root: &Path, repo_ir: &mut RepoIR) {
    let mut file_to_symbols: HashMap<String, Vec<Symbol>> = HashMap::new();
    let mut file_to_reexports: HashMap<String, Vec<ReExportEntry>> = HashMap::new();
    let known_files: HashSet<String> = repo_ir.files.iter().map(|f| f.file.clone()).collect();
    for file in &repo_ir.files {
        file_to_symbols.insert(file.file.clone(), file.symbols.clone());
        if !file.re_exports.is_empty() {
            file_to_reexports.insert(file.file.clone(), file.re_exports.clone());
        }
    }
    expand_star_reexports(root, &mut file_to_reexports, &file_to_symbols, &known_files);

    let tsconfig_paths = TsConfigPaths::load(root);

    for file_ir in &mut repo_ir.files {
        let mut resolved = Vec::new();
        let mut local_alias_to_symbol_id: HashMap<String, String> = HashMap::new();

        for relationship in &file_ir.relationships {
            if relationship.kind == RelationshipKind::Imports
                || relationship.kind == RelationshipKind::ReExports
            {
                let expansions = resolve_import_like(
                    root,
                    &file_ir.file,
                    relationship,
                    &file_to_symbols,
                    &file_to_reexports,
                    &known_files,
                    tsconfig_paths.as_ref(),
                    &mut file_ir.diagnostics,
                );
                for rel in expansions {
                    if relationship.kind == RelationshipKind::Imports {
                        let local_name = rel
                            .alias
                            .clone()
                            .unwrap_or_else(|| rel.to.split("::").nth(1).unwrap_or("").to_string());
                        if !local_name.is_empty() {
                            local_alias_to_symbol_id.insert(local_name, rel.to.clone());
                        }
                    }
                    resolved.push(rel);
                }
            } else {
                resolved.push(relationship.clone());
            }
        }

        for rel in &mut resolved {
            if rel.kind == RelationshipKind::Calls || rel.kind == RelationshipKind::Instantiates {
                if let Some(callee) = parse_unresolved_local_type_symbol_id(&rel.to) {
                    if let Some(canonical) = local_alias_to_symbol_id.get(&callee) {
                        rel.to = canonical.clone();
                    } else if let Some(found) =
                        find_symbol_by_name_in_file(&file_to_symbols, &file_ir.file, &callee)
                    {
                        rel.to = found;
                    }
                    // Anything still unresolved keeps its unresolved id and is
                    // dropped below. A call whose target is not a symbol this
                    // file can name — a global, a built-in, a value from an
                    // untyped module — is not a fact about the codebase graph,
                    // and no diagnostic is raised because there is nothing the
                    // user could fix. See the retain() after this loop.
                }
                continue;
            }
            if rel.kind != RelationshipKind::AccessesProperty
                && rel.kind != RelationshipKind::UsesType
            {
                continue;
            }
            if let Some(type_name) = parse_unresolved_local_type_symbol_id(&rel.to) {
                if let Some(canonical) = local_alias_to_symbol_id.get(&type_name) {
                    rel.to = canonical.clone();
                    continue;
                }
                if let Some(found) =
                    find_symbol_by_name_in_file(&file_to_symbols, &file_ir.file, &type_name)
                {
                    rel.to = found;
                    continue;
                }
                // Built-in types are not codebase symbols — silently skip
                if is_builtin_type(&type_name) {
                    continue;
                }
                file_ir.diagnostics.push(Diagnostic {
                    level: DiagnosticLevel::Warning,
                    category: DiagnosticCategory::Resolution,
                    message: format!(
                        "unable to resolve property-access type '{type_name}' in {}",
                        file_ir.file
                    ),
                    file: Some(file_ir.file.clone()),
                    line: None,
                });
            }
        }

        // A call or instantiation that never bound to a symbol is discarded
        // rather than recorded against a placeholder target. Keeping it would
        // put an edge in the graph pointing at a name rather than at a symbol,
        // and `blast-radius` would count it as a dependent.
        resolved.retain(|rel| {
            if rel.kind != RelationshipKind::Calls && rel.kind != RelationshipKind::Instantiates {
                return true;
            }
            parse_unresolved_local_type_symbol_id(&rel.to).is_none()
        });

        file_ir.relationships = resolved;
    }
}

fn expand_star_reexports(
    root: &Path,
    file_to_reexports: &mut HashMap<String, Vec<ReExportEntry>>,
    file_to_symbols: &HashMap<String, Vec<Symbol>>,
    known_files: &HashSet<String>,
) {
    let files: Vec<String> = file_to_reexports.keys().cloned().collect();
    for file in files {
        let entries = file_to_reexports.get(&file).cloned().unwrap_or_default();
        let mut expanded = Vec::new();
        for entry in entries {
            if entry.exported_name != "*" {
                expanded.push(entry);
                continue;
            }

            let Some(target_file) =
                resolve_target_file_from_known(root, &file, &entry.source_module, known_files)
            else {
                expanded.push(entry);
                continue;
            };

            let Some(symbols) = file_to_symbols.get(&target_file) else {
                expanded.push(entry);
                continue;
            };

            let mut added = false;
            for symbol in symbols {
                if symbol.name == "module" {
                    continue;
                }
                expanded.push(ReExportEntry {
                    exported_name: symbol.name.clone(),
                    source_module: entry.source_module.clone(),
                });
                added = true;
            }
            if !added {
                expanded.push(entry);
            }
        }
        file_to_reexports.insert(file, expanded);
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_import_like(
    root: &Path,
    file: &str,
    relationship: &Relationship,
    file_to_symbols: &HashMap<String, Vec<Symbol>>,
    file_to_reexports: &HashMap<String, Vec<ReExportEntry>>,
    known_files: &HashSet<String>,
    tsconfig_paths: Option<&TsConfigPaths>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Relationship> {
    let local_context = LocalImportContext {
        root,
        file,
        file_to_symbols,
        file_to_reexports,
        known_files,
    };

    let Some((module_specifier, symbol_name)) = parse_unresolved_import_symbol_id(&relationship.to)
    else {
        return vec![relationship.clone()];
    };

    // Non-relative specifiers: try tsconfig paths first, then classify as external
    match classify_module_specifier(&module_specifier) {
        ModuleKind::ExternalPackage | ModuleKind::NodeBuiltin => {
            // Try tsconfig paths before giving up to external
            if let Some(tc) = tsconfig_paths {
                if let Some(resolved_path) = tc.resolve(root, &module_specifier) {
                    let target_file = resolved_path.to_string_lossy().replace('\\', "/");
                    return resolve_local_import(
                        &local_context,
                        &target_file,
                        &symbol_name,
                        relationship,
                        diagnostics,
                    );
                }
            }
            let package_name = extract_package_name(&module_specifier);
            let external_id = format!("ext::{package_name}::package");
            let mut rel = relationship.clone();
            rel.to = external_id;
            return vec![rel];
        }
        ModuleKind::Relative | ModuleKind::Absolute => {}
    }

    let Some(target_file) =
        resolve_target_file_from_known(root, file, &module_specifier, known_files)
    else {
        diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Warning,
            category: DiagnosticCategory::Resolution,
            message: format!(
                "unresolved import target in {file}: {}",
                relationship.context
            ),
            file: Some(file.to_string()),
            line: Some(relationship.line),
        });
        return vec![relationship.clone()];
    };

    resolve_local_import(
        &local_context,
        &target_file,
        &symbol_name,
        relationship,
        diagnostics,
    )
}

fn resolve_local_import(
    context: &LocalImportContext<'_>,
    target_file: &str,
    symbol_name: &str,
    relationship: &Relationship,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Relationship> {
    let Some(target_symbols) = context.file_to_symbols.get(target_file) else {
        diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Warning,
            category: DiagnosticCategory::Resolution,
            message: format!("target file missing symbols for import resolution: {target_file}"),
            file: Some(context.file.to_string()),
            line: None,
        });
        return vec![relationship.clone()];
    };

    if symbol_name == "*" && relationship.kind == RelationshipKind::ReExports {
        let mut out = Vec::new();
        for symbol in target_symbols {
            if symbol.name == "module" {
                continue;
            }
            let mut rel = relationship.clone();
            rel.to = symbol.id.clone();
            out.push(rel);
        }
        if out.is_empty() {
            out.push(relationship.clone());
        }
        return out;
    }

    let resolved_id = if symbol_name == "default" {
        pick_default_export_candidate(target_symbols).map(|s| s.id.clone())
    } else {
        target_symbols
            .iter()
            .find(|s| s.name == symbol_name)
            .map(|s| s.id.clone())
    };

    // If not found directly, follow re-export chain (barrel files)
    let resolved_id = resolved_id.or_else(|| {
        let mut visited = HashSet::new();
        visited.insert(target_file.to_string());
        follow_reexport_chain(
            context.root,
            target_file,
            symbol_name,
            context.file_to_symbols,
            context.file_to_reexports,
            context.known_files,
            &mut visited,
            diagnostics,
        )
    });

    // The module is here and does not declare this name. An import still
    // created a dependency on the file, and that much is true — so record it,
    // the way the Rust adapter does for the same case, rather than dropping the
    // reference and reporting nothing.
    //
    // This is what a framework component import looks like. A Vue single-file
    // component *is* its file: `UserCard.vue` declares no symbol named
    // `UserCard`, so `import { UserCard } from './UserCard.vue'` could never
    // find one, and `blast-radius` on that component saw no dependents at all.
    // The same holds for `.astro`.
    //
    // Only for `Imports`. A re-export names a specific symbol it claims to
    // forward; when that symbol does not exist, forwarding nothing is the
    // honest answer and a file-level edge would overstate it.
    let resolved_id = resolved_id.or_else(|| {
        if relationship.kind != RelationshipKind::Imports {
            return None;
        }
        target_symbols
            .iter()
            .find(|s| s.name == "module")
            .map(|s| s.id.clone())
    });

    let Some(resolved_id) = resolved_id else {
        diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Warning,
            category: DiagnosticCategory::Resolution,
            message: format!("unable to resolve symbol '{symbol_name}' from {target_file}"),
            file: Some(context.file.to_string()),
            line: Some(relationship.line),
        });
        return vec![relationship.clone()];
    };

    let mut rel = relationship.clone();
    rel.to = resolved_id;
    vec![rel]
}

struct LocalImportContext<'a> {
    root: &'a Path,
    file: &'a str,
    file_to_symbols: &'a HashMap<String, Vec<Symbol>>,
    file_to_reexports: &'a HashMap<String, Vec<ReExportEntry>>,
    known_files: &'a HashSet<String>,
}

const MAX_REEXPORT_DEPTH: usize = 10;

#[allow(clippy::too_many_arguments)]
fn follow_reexport_chain(
    root: &Path,
    current_file: &str,
    symbol_name: &str,
    file_to_symbols: &HashMap<String, Vec<Symbol>>,
    file_to_reexports: &HashMap<String, Vec<ReExportEntry>>,
    known_files: &HashSet<String>,
    visited: &mut HashSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<SymbolId> {
    if visited.len() > MAX_REEXPORT_DEPTH {
        diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Warning,
            category: DiagnosticCategory::Resolution,
            message: format!(
                "re-export chain depth exceeded for '{symbol_name}' from {current_file}"
            ),
            file: Some(current_file.to_string()),
            line: None,
        });
        return None;
    }

    let reexports = file_to_reexports.get(current_file)?;

    // Helper to follow a specific entry
    let mut try_follow = |entry: &ReExportEntry| -> Option<SymbolId> {
        let chain_target =
            resolve_target_file_from_known(root, current_file, &entry.source_module, known_files)?;

        // Push visited
        if !visited.insert(chain_target.clone()) {
            diagnostics.push(Diagnostic {
                level: DiagnosticLevel::Warning,
                category: DiagnosticCategory::Resolution,
                message: format!(
                    "cyclic re-export detected for '{symbol_name}': {} → {chain_target}",
                    current_file
                ),
                file: Some(current_file.to_string()),
                line: None,
            });
            return None;
        }

        // Check if symbol exists in the direct target
        if let Some(chain_symbols) = file_to_symbols.get(&chain_target) {
            let found = if symbol_name == "default" {
                pick_default_export_candidate(chain_symbols).map(|s| s.id.clone())
            } else {
                chain_symbols
                    .iter()
                    .find(|s| s.name == symbol_name)
                    .map(|s| s.id.clone())
            };
            if found.is_some() {
                return found;
            }
        }

        // Recurse
        let result = follow_reexport_chain(
            root,
            &chain_target,
            symbol_name,
            file_to_symbols,
            file_to_reexports,
            known_files,
            visited,
            diagnostics,
        );

        // Pop visited to allow other branches to explore this target if needed
        // (Wait, standard DFS handles visited per path, here we use a shared visited set across all explorations
        // in this step to prevent cycles from exploding branching factor too. We can leave it inserted or pop it.
        // Popping is safer for false cycles across sibling paths.)
        visited.remove(&chain_target);

        result
    };

    // 1. Try exact matches first
    if let Some(exact_entry) = reexports.iter().find(|e| e.exported_name == symbol_name) {
        if let Some(id) = try_follow(exact_entry) {
            return Some(id);
        }
    }

    // 2. Try wildcard matches if exact match failed
    for star_entry in reexports.iter().filter(|e| e.exported_name == "*") {
        if let Some(id) = try_follow(star_entry) {
            return Some(id);
        }
    }

    None
}

fn pick_default_export_candidate(symbols: &[Symbol]) -> Option<&Symbol> {
    symbols
        .iter()
        .find(|s| s.name != "module" && !matches!(s.kind, openinvar_core::ir::SymbolKind::Property))
}

fn find_symbol_by_name_in_file(
    file_to_symbols: &HashMap<String, Vec<Symbol>>,
    file: &str,
    name: &str,
) -> Option<String> {
    let symbols = file_to_symbols.get(file)?;
    let mut candidates: Vec<&Symbol> = symbols
        .iter()
        .filter(|symbol| symbol.name == name)
        .collect();
    candidates.sort_by(|a, b| a.file.cmp(&b.file).then(a.id.cmp(&b.id)));
    candidates.first().map(|s| s.id.clone())
}

fn resolve_target_file_from_known(
    root: &Path,
    from_file: &str,
    module_specifier: &str,
    known_files: &HashSet<String>,
) -> Option<String> {
    if !module_specifier.starts_with('.') {
        return None;
    }

    let from_file_path = root.join(from_file);
    let base_dir = from_file_path.parent()?;
    let candidate = base_dir.join(module_specifier);

    let candidates = [
        candidate.clone(),
        append_extension(&candidate, "ts"),
        append_extension(&candidate, "tsx"),
        append_extension(&candidate, "mts"),
        append_extension(&candidate, "cts"),
        append_extension(&candidate, "js"),
        append_extension(&candidate, "jsx"),
        append_extension(&candidate, "mjs"),
        append_extension(&candidate, "cjs"),
        append_extension(&candidate, "vue"),
        append_extension(&candidate, "svelte"),
        append_extension(&candidate, "astro"),
        with_extension(&candidate, "ts"),
        with_extension(&candidate, "tsx"),
        with_extension(&candidate, "mts"),
        with_extension(&candidate, "cts"),
        with_extension(&candidate, "js"),
        with_extension(&candidate, "jsx"),
        with_extension(&candidate, "mjs"),
        with_extension(&candidate, "cjs"),
        with_extension(&candidate, "vue"),
        with_extension(&candidate, "svelte"),
        with_extension(&candidate, "astro"),
        candidate.join("index.ts"),
        candidate.join("index.tsx"),
        candidate.join("index.mts"),
        candidate.join("index.cts"),
        candidate.join("index.js"),
        candidate.join("index.jsx"),
        candidate.join("index.mjs"),
        candidate.join("index.cjs"),
        candidate.join("index.vue"),
        candidate.join("index.svelte"),
        candidate.join("index.astro"),
    ];

    for path in candidates {
        if let Some(rel) = relative_path_within_root(root, &path) {
            if known_files.contains(&rel) {
                return Some(rel);
            }
        }
    }

    None
}

fn with_extension(path: &Path, ext: &str) -> PathBuf {
    let mut out = path.to_path_buf();
    out.set_extension(ext);
    out
}

fn append_extension(path: &Path, ext: &str) -> PathBuf {
    PathBuf::from(format!("{}.{}", path.to_string_lossy(), ext))
}

fn relative_path_within_root(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    Some(normalize_relative_path(
        &rel.to_string_lossy().replace('\\', "/"),
    ))
}

fn normalize_relative_path(input: &str) -> String {
    let mut stack: Vec<&str> = Vec::new();
    for segment in input.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                let _ = stack.pop();
            }
            other => stack.push(other),
        }
    }
    stack.join("/")
}
