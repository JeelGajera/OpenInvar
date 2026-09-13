//! Locating `go.mod` files and mapping directories to package import paths.
//!
//! The previous version read `go.mod` only from the analysis root, so any
//! repository whose module lives in a subdirectory — a Go service inside a
//! polyglot monorepo, or any multi-module layout — resolved every import as
//! external and produced no local edges at all.
//!
//! Module lookup here follows Go's own rule: a file belongs to the module
//! declared by the nearest `go.mod` at or above its directory.

use std::collections::HashMap;
use std::path::Path;

/// One `go.mod` and the directory it governs.
#[derive(Debug, Clone)]
pub struct GoModule {
    /// The `module` line, e.g. `github.com/test/app`.
    pub module_path: String,
    /// Directory containing the `go.mod`, relative to the analysis root.
    pub dir: String,
}

/// Every module found in the tree, indexed for nearest-ancestor lookup.
#[derive(Debug, Default)]
pub struct ModuleSet {
    /// Relative directory of a `go.mod` → the module it declares.
    by_dir: HashMap<String, GoModule>,
}

impl ModuleSet {
    /// Discover the modules covering `files`.
    ///
    /// Only directories that actually contain Go sources are probed, so this
    /// costs a handful of `stat` calls rather than a full tree walk.
    pub fn discover(root: &Path, files: &[String]) -> Self {
        let mut set = Self::default();
        let mut probed: HashMap<String, bool> = HashMap::new();

        for file in files {
            let mut dir = parent_dir(file);
            loop {
                if !probed.contains_key(&dir) {
                    let candidate = if dir.is_empty() {
                        root.join("go.mod")
                    } else {
                        root.join(&dir).join("go.mod")
                    };
                    let found = match std::fs::read_to_string(&candidate) {
                        Ok(content) => match parse_module_path(&content) {
                            Some(module_path) => {
                                set.by_dir.insert(
                                    dir.clone(),
                                    GoModule {
                                        module_path,
                                        dir: dir.clone(),
                                    },
                                );
                                true
                            }
                            None => false,
                        },
                        Err(_) => false,
                    };
                    probed.insert(dir.clone(), found);
                }

                if dir.is_empty() {
                    break;
                }
                dir = parent_dir(&dir);
            }
        }

        set
    }

    /// True if no `go.mod` was found anywhere.
    pub fn is_empty(&self) -> bool {
        self.by_dir.is_empty()
    }

    /// The module governing `file`: the nearest `go.mod` at or above it.
    pub fn module_for(&self, file: &str) -> Option<&GoModule> {
        let mut dir = parent_dir(file);
        loop {
            if let Some(module) = self.by_dir.get(&dir) {
                return Some(module);
            }
            if dir.is_empty() {
                return None;
            }
            dir = parent_dir(&dir);
        }
    }

    /// The canonical import path of the package `file` belongs to.
    ///
    /// A package's import path is its module path joined with its directory
    /// relative to the module root.
    pub fn import_path_for(&self, file: &str) -> Option<String> {
        let module = self.module_for(file)?;
        let dir = parent_dir(file);
        let relative = dir
            .strip_prefix(&module.dir)
            .map(|r| r.trim_start_matches('/'))
            .unwrap_or(&dir);

        Some(if relative.is_empty() {
            module.module_path.clone()
        } else {
            format!("{}/{}", module.module_path, relative)
        })
    }

    /// True if `import_path` resolves inside one of the discovered modules.
    pub fn is_local_import(&self, import_path: &str) -> bool {
        self.by_dir.values().any(|m| {
            import_path == m.module_path || import_path.starts_with(&format!("{}/", m.module_path))
        })
    }
}

fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(cut) => path[..cut].to_string(),
        None => String::new(),
    }
}

/// Read the `module` declaration out of a `go.mod`.
fn parse_module_path(content: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim();
        // `module` may be followed by a space or a tab.
        let Some(rest) = line.strip_prefix("module") else {
            continue;
        };
        if !rest.starts_with(char::is_whitespace) {
            continue;
        }
        let path = rest.trim().trim_matches('"').trim();
        if !path.is_empty() {
            return Some(path.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_paths_are_read_from_go_mod() {
        assert_eq!(
            parse_module_path("module github.com/test/app\n\ngo 1.22\n"),
            Some("github.com/test/app".to_string())
        );
        assert_eq!(
            parse_module_path("// comment\nmodule\tgithub.com/x/y\n"),
            Some("github.com/x/y".to_string())
        );
        // `modules` must not be mistaken for `module`.
        assert_eq!(parse_module_path("modulefoo bar\n"), None);
        assert_eq!(parse_module_path("go 1.22\n"), None);
    }

    #[test]
    fn nearest_module_wins_for_nested_layouts() {
        let mut set = ModuleSet::default();
        set.by_dir.insert(
            String::new(),
            GoModule {
                module_path: "github.com/test/root".into(),
                dir: String::new(),
            },
        );
        set.by_dir.insert(
            "services/api".into(),
            GoModule {
                module_path: "github.com/test/api".into(),
                dir: "services/api".into(),
            },
        );

        assert_eq!(
            set.import_path_for("services/api/handlers/user.go")
                .unwrap(),
            "github.com/test/api/handlers"
        );
        assert_eq!(
            set.import_path_for("internal/util/util.go").unwrap(),
            "github.com/test/root/internal/util"
        );
        assert_eq!(
            set.import_path_for("services/api/main.go").unwrap(),
            "github.com/test/api"
        );
    }
}
