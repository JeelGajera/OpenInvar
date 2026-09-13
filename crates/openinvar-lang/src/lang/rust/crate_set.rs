//! Discovering the Cargo packages in a tree and the crate roots they declare.
//!
//! Resolution used to assume one crate rooted at `<repo>/src/`. It derived a
//! module path by stripping a leading `src/` and prefixing `crate::`, so in a
//! workspace `crates/openinvar-core/src/ir.rs` became
//! `crate::crates::openinvar-core::src::ir`, nothing matched, and every
//! `use openinvar_core::…` fell through to "external". OpenInvar could not analyze
//! itself: `usages RepoIR` returned nothing and `blast-radius RepoIR` reported
//! it safe to modify.
//!
//! The same assumption also broke *nested* crates that are not workspace
//! members — this repository's own Rust fixtures, each a small crate under
//! `fixtures/`, were folded into one imaginary crate alongside the real code.
//! So discovery here is by `Cargo.toml` found anywhere in the tree, not by
//! workspace membership alone.
//!
//! This mirrors the Go adapter's `ModuleSet`, which already solves the
//! equivalent problem by finding the nearest `go.mod` at or above a file.
//!
//! # Naming
//!
//! Every module path is made absolute against the *crate* that owns it, and
//! the namespace is the name the crate is known by to other crates — its
//! package name with hyphens turned into underscores, or its `[lib] name` if
//! it sets one. So `crates/openinvar-core/src/ir.rs` is `openinvar_core::ir`,
//! whether it is reached from inside the crate as `crate::ir` or from another
//! crate as `openinvar_core::ir`. Intra-crate and inter-crate resolution then
//! become the same lookup rather than two mechanisms that must agree.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// What a crate root is for. Only `Lib` is reachable from other crates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RootKind {
    Lib,
    Bin,
    Test,
    Bench,
    Example,
}

/// One file that begins a module tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateRoot {
    /// The root file, relative to the analysis root.
    pub file: String,
    /// Directory whose contents are modules of this root.
    ///
    /// For `pkg/src/lib.rs` this is `pkg/src`; for `pkg/src/bin/tool.rs` it is
    /// `pkg/src/bin/tool`, which is where Rust looks for that binary's
    /// submodules.
    pub module_dir: String,
    /// The namespace every module under this root is addressed by.
    ///
    /// The crate's extern name for a `Lib` root. Other root kinds get a
    /// suffixed namespace of their own: a binary's `crate::` is not the
    /// library's `crate::`, and nothing outside the package can name either.
    pub namespace: String,
    pub kind: RootKind,
    /// Index into [`CrateSet::crates`].
    pub package: usize,
}

/// One Cargo package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CratePackage {
    /// The name other crates use in a `use` path: `openinvar-core` is
    /// `openinvar_core`, overridden by `[lib] name` when present.
    pub extern_name: String,
    /// Directory holding the `Cargo.toml`, relative to the analysis root.
    pub dir: String,
    /// Dependency renames: the local name in a `use` path, and the package it
    /// actually refers to. `foo = { package = "bar" }` means `use foo::X` is
    /// really `bar`'s `X`.
    pub renames: BTreeMap<String, String>,
}

/// Every package found in the tree, with its crate roots.
#[derive(Debug, Default)]
pub struct CrateSet {
    pub crates: Vec<CratePackage>,
    /// Sorted by `module_dir` length descending, so the most specific root
    /// that could own a file is found first.
    roots: Vec<CrateRoot>,
    by_extern_name: BTreeMap<String, usize>,
}

impl CrateSet {
    /// Find the packages covering `files`.
    ///
    /// Only directories that actually contain Rust sources are probed, so this
    /// costs a handful of reads rather than a full tree walk — the same
    /// approach the Go adapter takes to `go.mod`.
    pub fn discover(root: &Path, files: &[String]) -> Self {
        let mut manifests: BTreeSet<String> = BTreeSet::new();

        // The nearest Cargo.toml at or above each source file.
        for file in files {
            let mut dir = parent_dir(file);
            loop {
                if manifest_exists(root, &dir) {
                    manifests.insert(dir.clone());
                    break;
                }
                if dir.is_empty() {
                    break;
                }
                dir = parent_dir(&dir);
            }
        }

        // A workspace root often declares members whose sources are present
        // even when nothing above pointed at them, and it may carry no
        // package of its own. Expanding members here also picks up the
        // `[lib] name` and rename tables that inter-crate resolution needs.
        let mut queue: Vec<String> = manifests.iter().cloned().collect();
        let mut seen: BTreeSet<String> = manifests.clone();
        while let Some(dir) = queue.pop() {
            for member in workspace_members(root, &dir) {
                if seen.insert(member.clone()) {
                    queue.push(member.clone());
                    manifests.insert(member);
                }
            }
        }

        let mut set = Self::default();
        for dir in manifests {
            set.add_package(root, &dir, files);
        }

        set.add_conventional_roots(root, files);

        // Longest module_dir first: `pkg/src/bin/tool` must win over
        // `pkg/src` for a file inside it.
        set.roots.sort_by(|a, b| {
            b.module_dir
                .len()
                .cmp(&a.module_dir.len())
                .then(a.file.cmp(&b.file))
        });
        set
    }

    /// Recognise a crate root that has no manifest beside it.
    ///
    /// A `src/lib.rs` is a crate root whether or not a `Cargo.toml` sits next
    /// to it. Requiring the manifest would drop every tree that does not ship
    /// one — this repository's own Rust fixtures among them — and, worse,
    /// drop them silently: with no root, a file has no module path, so
    /// `crate::models::user` matches nothing and is classified as a
    /// third-party crate instead of reported as unresolved. A wrong answer
    /// with no diagnostic is the one outcome to avoid.
    ///
    /// The namespace comes from the directory name, so a fixture analyzed on
    /// its own and the same fixture analyzed as part of a larger tree both
    /// resolve their own `crate::` paths correctly.
    fn add_conventional_roots(&mut self, root: &Path, files: &[String]) {
        // Libraries first: `src/lib.rs` and `src/main.rs` share a module
        // directory, and a module under `src/` is far more often the
        // library's.
        for (suffix, kind) in [
            ("src/lib.rs", RootKind::Lib),
            ("src/main.rs", RootKind::Bin),
        ] {
            for file in files {
                let pkg_dir = if file == suffix {
                    String::new()
                } else if let Some(dir) = file.strip_suffix(&format!("/{suffix}")) {
                    dir.to_string()
                } else {
                    continue;
                };

                // A manifest already described this package properly.
                if manifest_exists(root, &pkg_dir) {
                    continue;
                }
                if self.roots.iter().any(|r| r.file == *file) {
                    continue;
                }

                let extern_name = match pkg_dir.rsplit('/').next() {
                    Some(name) if !name.is_empty() => normalize_crate_name(name),
                    // The analysis root is the crate: it has no directory name
                    // to borrow, and `crate` is what its own paths say.
                    _ => "crate".to_string(),
                };

                let index = self.crates.len();
                self.crates.push(CratePackage {
                    extern_name: extern_name.clone(),
                    dir: pkg_dir.clone(),
                    renames: BTreeMap::new(),
                });
                self.by_extern_name
                    .entry(extern_name.clone())
                    .or_insert(index);
                self.push_root(&pkg_dir, suffix, kind, &extern_name, index);
            }
        }
    }

    fn add_package(&mut self, root: &Path, dir: &str, files: &[String]) {
        let Some(manifest) = read_manifest(root, dir) else {
            return;
        };
        let Some(package) = manifest.get("package").and_then(|p| p.as_table()) else {
            // A virtual manifest — a workspace root with no package of its
            // own. Its members were queued separately.
            return;
        };
        let Some(name) = package.get("name").and_then(|n| n.as_str()) else {
            return;
        };

        let lib = manifest.get("lib").and_then(|l| l.as_table());
        let extern_name = lib
            .and_then(|l| l.get("name"))
            .and_then(|n| n.as_str())
            .map(normalize_crate_name)
            .unwrap_or_else(|| normalize_crate_name(name));

        let index = self.crates.len();
        self.crates.push(CratePackage {
            extern_name: extern_name.clone(),
            dir: dir.to_string(),
            renames: dependency_renames(&manifest),
        });
        // First package wins a duplicated name; two packages exporting the
        // same extern name cannot both be addressed anyway.
        self.by_extern_name
            .entry(extern_name.clone())
            .or_insert(index);

        // ── explicit targets ─────────────────────────────────
        if let Some(path) = lib.and_then(|l| l.get("path")).and_then(|p| p.as_str()) {
            self.push_root(dir, path, RootKind::Lib, &extern_name, index);
        } else if file_exists(root, &join(dir, "src/lib.rs")) {
            self.push_root(dir, "src/lib.rs", RootKind::Lib, &extern_name, index);
        }

        for (key, kind) in [
            ("bin", RootKind::Bin),
            ("test", RootKind::Test),
            ("bench", RootKind::Bench),
            ("example", RootKind::Example),
        ] {
            for target in manifest
                .get(key)
                .and_then(|t| t.as_array())
                .map(|a| a.as_slice())
                .unwrap_or(&[])
            {
                if let Some(path) = target.get("path").and_then(|p| p.as_str()) {
                    self.push_root(dir, path, kind, &extern_name, index);
                }
            }
        }

        // ── conventional targets ─────────────────────────────
        //
        // Cargo finds these without them being declared, so a repository that
        // declares nothing still resolves.
        if file_exists(root, &join(dir, "src/main.rs")) {
            self.push_root(dir, "src/main.rs", RootKind::Bin, &extern_name, index);
        }
        for (subdir, kind) in [
            ("src/bin", RootKind::Bin),
            ("tests", RootKind::Test),
            ("benches", RootKind::Bench),
            ("examples", RootKind::Example),
        ] {
            let prefix = join(dir, subdir);
            for file in files {
                let Some(rest) = file.strip_prefix(&format!("{prefix}/")) else {
                    continue;
                };
                // Only files directly in the directory root a target;
                // anything deeper is a submodule of one.
                let is_direct_rs = rest.ends_with(".rs") && !rest.contains('/');
                // `tests/thing/main.rs` is also a target root.
                let is_dir_main = rest.ends_with("/main.rs") && rest.matches('/').count() == 1;
                if is_direct_rs || is_dir_main {
                    self.push_root(dir, &join(subdir, rest), kind, &extern_name, index);
                }
            }
        }
    }

    fn push_root(
        &mut self,
        pkg_dir: &str,
        relative: &str,
        kind: RootKind,
        extern_name: &str,
        package: usize,
    ) {
        let file = join(pkg_dir, relative);
        if self.roots.iter().any(|r| r.file == file) {
            return;
        }

        // A lib root's modules live beside it; every other root's live in a
        // directory named after the target.
        let module_dir = match kind {
            RootKind::Lib => parent_dir(&file),
            _ => file.strip_suffix(".rs").unwrap_or(&file).to_string(),
        };

        // `src/main.rs` is a special case: Cargo looks for its submodules in
        // `src/`, not `src/main/`.
        let module_dir = if file.ends_with("/main.rs") || file == "main.rs" {
            parent_dir(&file)
        } else {
            module_dir
        };

        let namespace = match kind {
            RootKind::Lib => extern_name.to_string(),
            // Nothing outside the package can name a binary's or a test's
            // root module, so the namespace only has to be unique.
            _ => format!("{extern_name}#{}", file),
        };

        self.roots.push(CrateRoot {
            file,
            module_dir,
            namespace,
            kind,
            package,
        });
    }

    /// The root whose module tree contains `file`.
    ///
    /// A file that is itself a root belongs to that root. Otherwise the most
    /// specific enclosing `module_dir` wins, and among equally specific ones a
    /// library beats a binary: `src/lib.rs` and `src/main.rs` both claim
    /// `src/`, and a module under `src/` is far more often the library's.
    pub fn root_for(&self, file: &str) -> Option<&CrateRoot> {
        if let Some(exact) = self.roots.iter().find(|r| r.file == file) {
            return Some(exact);
        }
        let mut best: Option<&CrateRoot> = None;
        for root in &self.roots {
            if !is_within(file, &root.module_dir) {
                continue;
            }
            let better = match best {
                None => true,
                Some(current) => {
                    root.module_dir.len() > current.module_dir.len()
                        || (root.module_dir.len() == current.module_dir.len()
                            && root.kind < current.kind)
                }
            };
            if better {
                best = Some(root);
            }
        }
        best
    }

    /// The absolute module path of `file`, namespaced by its crate.
    ///
    /// `crates/openinvar-core/src/ir.rs` is `openinvar_core::ir`; the crate root
    /// itself is bare `openinvar_core`.
    pub fn module_path_for(&self, file: &str) -> Option<String> {
        let root = self.root_for(file)?;
        if root.file == file {
            return Some(root.namespace.clone());
        }

        let rest = file
            .strip_prefix(&format!("{}/", root.module_dir))
            .unwrap_or(file);
        let rest = rest.strip_suffix(".rs").unwrap_or(rest);
        let rest = rest.strip_suffix("/mod").unwrap_or(rest);
        if rest.is_empty() {
            return Some(root.namespace.clone());
        }
        Some(format!("{}::{}", root.namespace, rest.replace('/', "::")))
    }

    /// Resolve the first segment of a `use` path written inside `from_file`.
    ///
    /// Returns the namespace it names, honouring this package's dependency
    /// renames. `None` means no crate in the tree goes by that name, which is
    /// how a third-party dependency is recognised.
    pub fn extern_namespace(&self, from_file: &str, first_segment: &str) -> Option<String> {
        let renamed = self
            .root_for(from_file)
            .and_then(|root| self.crates.get(root.package))
            .and_then(|pkg| pkg.renames.get(first_segment))
            .cloned();

        let name = renamed.unwrap_or_else(|| first_segment.to_string());
        self.by_extern_name
            .get(&name)
            .and_then(|i| self.crates.get(*i))
            .map(|c| c.extern_name.clone())
    }

    /// True when `namespace` belongs to a crate in this tree.
    pub fn is_local_namespace(&self, namespace: &str) -> bool {
        let head = namespace.split("::").next().unwrap_or(namespace);
        self.roots.iter().any(|r| r.namespace == head)
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    #[cfg(test)]
    pub fn roots(&self) -> &[CrateRoot] {
        &self.roots
    }
}

// ── manifest reading ─────────────────────────────────────────

fn manifest_exists(root: &Path, dir: &str) -> bool {
    file_exists(root, &join(dir, "Cargo.toml"))
}

fn file_exists(root: &Path, relative: &str) -> bool {
    root.join(relative).is_file()
}

fn read_manifest(root: &Path, dir: &str) -> Option<toml::Table> {
    let text = std::fs::read_to_string(root.join(join(dir, "Cargo.toml"))).ok()?;
    text.parse::<toml::Table>().ok()
}

/// Member directories of the workspace declared at `dir`, if any.
///
/// Globs are expanded against the filesystem and `exclude` is honoured, both
/// because Cargo does and because a workspace that lists `crates/*` is the
/// common case.
fn workspace_members(root: &Path, dir: &str) -> Vec<String> {
    let Some(manifest) = read_manifest(root, dir) else {
        return Vec::new();
    };
    let Some(workspace) = manifest.get("workspace").and_then(|w| w.as_table()) else {
        return Vec::new();
    };

    let list = |key: &str| -> Vec<String> {
        workspace
            .get(key)
            .and_then(|m| m.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| join(dir, s))
                    .collect()
            })
            .unwrap_or_default()
    };

    let excluded: BTreeSet<String> = list("exclude")
        .into_iter()
        .flat_map(|p| expand(root, &p))
        .collect();

    let mut members: Vec<String> = list("members")
        .into_iter()
        .flat_map(|p| expand(root, &p))
        .filter(|m| !excluded.contains(m))
        .collect();
    members.sort();
    members.dedup();
    members
}

/// Expand a member pattern to the directories that exist and hold a manifest.
fn expand(root: &Path, pattern: &str) -> Vec<String> {
    if !pattern.contains('*') && !pattern.contains('?') && !pattern.contains('[') {
        return if manifest_exists(root, pattern) {
            vec![pattern.to_string()]
        } else {
            Vec::new()
        };
    }

    let absolute = root.join(pattern);
    let Ok(paths) = glob::glob(&absolute.to_string_lossy()) else {
        return Vec::new();
    };

    let mut out: Vec<String> = paths
        .flatten()
        .filter_map(|p| {
            let relative = p.strip_prefix(root).ok()?;
            let relative = relative.to_string_lossy().replace('\\', "/");
            manifest_exists(root, &relative).then_some(relative)
        })
        .collect();
    // glob's iteration order is filesystem order; sort so discovery is
    // reproducible, which the whole product depends on.
    out.sort();
    out
}

/// `foo = { package = "bar" }` — the name a dependency is used under, and the
/// package it actually is.
fn dependency_renames(manifest: &toml::Table) -> BTreeMap<String, String> {
    let mut renames = BTreeMap::new();
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let Some(table) = manifest.get(section).and_then(|d| d.as_table()) else {
            continue;
        };
        for (local, spec) in table {
            if let Some(real) = spec.get("package").and_then(|p| p.as_str()) {
                renames.insert(normalize_crate_name(local), normalize_crate_name(real));
            }
        }
    }
    renames
}

/// The identifier a crate is known by in source: hyphens become underscores.
pub fn normalize_crate_name(name: &str) -> String {
    name.replace('-', "_")
}

// ── path helpers ─────────────────────────────────────────────

fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(cut) => path[..cut].to_string(),
        None => String::new(),
    }
}

fn join(base: &str, rest: &str) -> String {
    let rest = rest.trim_start_matches("./");
    if base.is_empty() {
        rest.to_string()
    } else if rest.is_empty() {
        base.to_string()
    } else {
        format!("{}/{}", base.trim_end_matches('/'), rest)
    }
}

/// True when `file` sits inside `dir` — not merely shares a prefix with it.
///
/// A plain `starts_with` would put `src/models_v2/x.rs` inside `src/models`.
fn is_within(file: &str, dir: &str) -> bool {
    if dir.is_empty() {
        return true;
    }
    file.strip_prefix(dir)
        .is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_names_become_identifiers() {
        assert_eq!(normalize_crate_name("openinvar-core"), "openinvar_core");
        assert_eq!(normalize_crate_name("serde_json"), "serde_json");
    }

    #[test]
    fn containment_requires_a_path_boundary() {
        assert!(is_within("src/models/user.rs", "src/models"));
        assert!(!is_within("src/models_v2/user.rs", "src/models"));
        assert!(!is_within("src/models", "src/models"));
        assert!(is_within("anything.rs", ""));
    }

    #[test]
    fn the_workspace_fixture_is_discovered_as_several_crates() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/adapter-rust/workspace");
        let files: Vec<String> = [
            "crates/app/src/lib.rs",
            "crates/app/src/main.rs",
            "crates/app/src/service.rs",
            "crates/core-lib/src/lib.rs",
            "crates/core-lib/src/models/mod.rs",
            "crates/core-lib/src/models/payload.rs",
            "crates/legacy/src/lib.rs",
            "crates/renamed-lib/src/lib.rs",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let set = CrateSet::discover(&root, &files);

        let mut names: Vec<&str> = set.crates.iter().map(|c| c.extern_name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            // `core-lib` normalises, `renamed-lib` reports its [lib] name, and
            // the excluded `legacy` is still a package of its own.
            vec!["app", "core_lib", "kernel", "legacy"]
        );

        // A binary and a library in one package are separate roots sharing a
        // module directory.
        let app_roots: Vec<&CrateRoot> = set
            .roots()
            .iter()
            .filter(|r| r.file.starts_with("crates/app/"))
            .collect();
        assert_eq!(app_roots.len(), 2);

        // A module under `src/` belongs to the library, not the binary.
        let owner = set
            .root_for("crates/app/src/service.rs")
            .expect("service.rs has an owning root");
        assert_eq!(owner.kind, RootKind::Lib);
        assert_eq!(owner.namespace, "app");
    }

    #[test]
    fn module_paths_are_namespaced_by_their_crate() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/adapter-rust/workspace");
        let files: Vec<String> = [
            "crates/core-lib/src/lib.rs",
            "crates/core-lib/src/models/mod.rs",
            "crates/core-lib/src/models/payload.rs",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let set = CrateSet::discover(&root, &files);

        // The crate root is the bare namespace; `mod.rs` names its directory.
        assert_eq!(
            set.module_path_for("crates/core-lib/src/lib.rs").unwrap(),
            "core_lib"
        );
        assert_eq!(
            set.module_path_for("crates/core-lib/src/models/mod.rs")
                .unwrap(),
            "core_lib::models"
        );
        assert_eq!(
            set.module_path_for("crates/core-lib/src/models/payload.rs")
                .unwrap(),
            "core_lib::models::payload"
        );
    }

    #[test]
    fn a_tree_with_no_manifest_falls_back_to_the_conventional_root() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/adapter-rust/alias_import_bug");
        let files: Vec<String> = ["src/lib.rs", "src/models/user_payload.rs"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        let set = CrateSet::discover(&root, &files);
        assert!(
            !set.is_empty(),
            "a src/lib.rs is a crate root without a manifest"
        );
        // The analysis root has no directory name to borrow, so its own paths
        // are what `crate::` says.
        assert_eq!(set.module_path_for("src/lib.rs").unwrap(), "crate");
        assert_eq!(
            set.module_path_for("src/models/user_payload.rs").unwrap(),
            "crate::models::user_payload"
        );
    }

    #[test]
    fn paths_join_without_doubling_separators() {
        assert_eq!(join("", "src/lib.rs"), "src/lib.rs");
        assert_eq!(join("crates/core", "src/lib.rs"), "crates/core/src/lib.rs");
        assert_eq!(
            join("crates/core", "./src/lib.rs"),
            "crates/core/src/lib.rs"
        );
        assert_eq!(join("crates/core/", "src/lib.rs"), "crates/core/src/lib.rs");
    }
}
