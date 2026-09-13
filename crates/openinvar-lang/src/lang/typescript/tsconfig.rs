use std::path::{Path, PathBuf};

type PathMappings = Vec<(String, Vec<String>)>;
type TsConfigChain = (PathBuf, PathMappings);

/// Resolved tsconfig path alias mappings.
pub struct TsConfigPaths {
    base_url: PathBuf,
    /// Sorted by pattern specificity (longest prefix first).
    /// Each entry: (prefix_before_star, suffix_after_star, replacement_prefixes)
    paths: Vec<(String, String, Vec<String>)>,
}

impl TsConfigPaths {
    /// Load tsconfig.json from the repo root, following `extends` chains.
    pub fn load(root: &Path) -> Option<Self> {
        let tsconfig_path = find_tsconfig(root)?;
        let (base_url, paths) = load_tsconfig_chain(&tsconfig_path, root, 5)?;

        if paths.is_empty() && base_url == *root {
            return None; // nothing useful to resolve
        }

        let mut entries: Vec<(String, String, Vec<String>)> = Vec::new();
        for (pattern, replacements) in &paths {
            let (prefix, suffix) = match pattern.find('*') {
                Some(pos) => (pattern[..pos].to_string(), pattern[pos + 1..].to_string()),
                None => (pattern.clone(), String::new()),
            };
            let repl_prefixes: Vec<String> = replacements
                .iter()
                .map(|r| {
                    // Strip the trailing * from replacement if present
                    match r.find('*') {
                        Some(pos) => r[..pos].to_string(),
                        None => r.clone(),
                    }
                })
                .collect();
            entries.push((prefix, suffix, repl_prefixes));
        }
        // Sort longest prefix first for greedy matching
        entries.sort_by_key(|b| std::cmp::Reverse(b.0.len()));

        Some(Self {
            base_url,
            paths: entries,
        })
    }

    /// Try to resolve a module specifier through tsconfig path aliases.
    /// Returns the resolved relative path (relative to root) if matched.
    pub fn resolve(&self, root: &Path, specifier: &str) -> Option<PathBuf> {
        for (prefix, suffix, replacements) in &self.paths {
            if !specifier.starts_with(prefix.as_str()) {
                continue;
            }
            if !suffix.is_empty() && !specifier.ends_with(suffix.as_str()) {
                continue;
            }

            // Extract the wildcard match
            let wildcard = &specifier[prefix.len()..specifier.len() - suffix.len()];

            for repl_prefix in replacements {
                let resolved = format!("{repl_prefix}{wildcard}{suffix}");
                let candidate = self.base_url.join(&resolved);

                // Try the candidate with standard TS extension probing
                if let Some(found) = probe_ts_file(root, &candidate) {
                    return Some(found);
                }
            }
        }

        // Also try baseUrl-relative resolution (no paths match needed)
        if self.base_url != *root {
            let candidate = self.base_url.join(specifier);
            if let Some(found) = probe_ts_file(root, &candidate) {
                return Some(found);
            }
        }

        None
    }
}

/// Probe for a TypeScript/JavaScript file at the given candidate path.
fn probe_ts_file(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let candidates = [
        candidate.to_path_buf(),
        candidate.with_extension("ts"),
        candidate.with_extension("tsx"),
        candidate.with_extension("mts"),
        candidate.with_extension("cts"),
        candidate.with_extension("js"),
        candidate.with_extension("jsx"),
        candidate.with_extension("mjs"),
        candidate.with_extension("cjs"),
        candidate.with_extension("vue"),
        candidate.with_extension("svelte"),
        candidate.with_extension("astro"),
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

    let root_canon = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());

    for path in &candidates {
        if path.is_file() {
            let path_canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            if let Ok(stripped) = path_canon.strip_prefix(&root_canon) {
                return Some(stripped.to_path_buf());
            }
            // Fallback if canonicalization failed or didn't strip for some reason
            return path.strip_prefix(root).ok().map(|p| p.to_path_buf());
        }
    }
    None
}

/// Find the nearest tsconfig.json starting from root.
fn find_tsconfig(root: &Path) -> Option<PathBuf> {
    let path = root.join("tsconfig.json");
    if path.is_file() {
        return Some(path);
    }
    None
}

/// Load a tsconfig.json, following `extends` chains up to max_depth.
fn load_tsconfig_chain(
    tsconfig_path: &Path,
    root: &Path,
    max_depth: usize,
) -> Option<TsConfigChain> {
    if max_depth == 0 {
        return None;
    }

    let content = std::fs::read_to_string(tsconfig_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;

    let tsconfig_dir = tsconfig_path.parent().unwrap_or(root);

    // Start with base config if extends is present
    let (mut base_url, mut paths) =
        if let Some(extends) = json.get("extends").and_then(|v| v.as_str()) {
            let parent_path = tsconfig_dir.join(extends);
            // If extends points to a directory, try tsconfig.json inside it
            let parent_path = if parent_path.is_dir() {
                parent_path.join("tsconfig.json")
            } else if parent_path.extension().is_none() {
                parent_path.with_extension("json")
            } else {
                parent_path
            };

            if parent_path.is_file() {
                load_tsconfig_chain(&parent_path, root, max_depth - 1)
                    .unwrap_or_else(|| (root.to_path_buf(), Vec::new()))
            } else {
                (root.to_path_buf(), Vec::new())
            }
        } else {
            (root.to_path_buf(), Vec::new())
        };

    // Override with this config's compilerOptions
    if let Some(compiler_options) = json.get("compilerOptions") {
        if let Some(bu) = compiler_options.get("baseUrl").and_then(|v| v.as_str()) {
            base_url = std::fs::canonicalize(tsconfig_dir.join(bu))
                .unwrap_or_else(|_| tsconfig_dir.join(bu));
        }

        if let Some(p) = compiler_options.get("paths").and_then(|v| v.as_object()) {
            let mut new_paths: Vec<(String, Vec<String>)> = Vec::new();
            for (pattern, values) in p {
                if let Some(arr) = values.as_array() {
                    let replacements: Vec<String> = arr
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect();
                    new_paths.push((pattern.clone(), replacements));
                }
            }
            // Child overrides parent paths
            let existing_keys: std::collections::HashSet<_> =
                new_paths.iter().map(|(k, _)| k.clone()).collect();
            paths.retain(|(k, _)| !existing_keys.contains(k));
            paths.extend(new_paths);
        }
    }

    Some((base_url, paths))
}
