pub mod analyze;
pub mod audit;
pub mod check;
pub mod context;
pub mod diff;
pub mod impact;
pub mod json;
pub mod markdown;
pub mod query;
pub mod report;
pub mod serve;
pub mod status;
pub mod tests;
pub mod watch;
pub mod worktree;

use std::path::{Path, PathBuf};

use openinvar_core::audit::Suppressions;
use openinvar_core::rules::{self, Located};

/// Normalize canonicalized paths for OpenInvar command usage.
///
/// On Windows, `canonicalize` can return extended-length paths (`\\?\...` or
/// `\\?\UNC\...`) that are problematic for downstream path consumers.
pub fn normalize_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let s = path.to_string_lossy();
        let stripped = if s.starts_with(r"\\?\UNC\") {
            format!(r"\\{}", &s[8..])
        } else if s.starts_with(r"\\?\") {
            s[4..].to_string()
        } else {
            s.into_owned()
        };
        PathBuf::from(stripped.replace('/', "\\"))
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

/// Convention: the graph database lives at `<repo_root>/.openinvar/graph.db`
///
/// A file, not a directory — the store is SQLite. Releases before this one put
/// a RocksDB directory at `<repo_root>/.openinvar/db`, and one of those may
/// still be sitting there; see [`legacy_store_dir`].
pub fn db_path(repo_root: &Path) -> PathBuf {
    normalize_path(repo_root).join(".openinvar").join("graph.db")
}

/// A store directory left by a release that used RocksDB, if one is here.
///
/// Checked so a command can say *why* there is no graph rather than only that
/// there is none. Both the current data directory and the one the project used
/// before it was renamed are looked at, because a repository analysed by an
/// older release carries `.graphyn/db` and nothing has told its owner that the
/// name moved.
///
/// Nothing here deletes anything. The graph is derived data and rebuilding it
/// takes under a second, but the directory is still the user's to remove.
pub fn legacy_store_dir(repo_root: &Path) -> Option<PathBuf> {
    let root = normalize_path(repo_root);
    [root.join(".openinvar").join("db"), root.join(".graphyn").join("db")]
        .into_iter()
        .find(|dir| openinvar_store::sqlite::is_legacy_rocksdb_store(dir))
}

/// The line shown when configuration is read from the pre-0.3.0 location.
///
/// Warned rather than ignored. Silently dropping a repository's rules on
/// upgrade would be a gate that stopped enforcing without saying so, which is
/// the failure this project is built against.
pub fn legacy_config_notice(found: &Path, root: &Path) -> String {
    format!(
        "{} is deprecated and will stop being read in 1.0.0. Move it to {} — \
         the old location is inside the gitignored data directory, so it could \
         never be committed.",
        found.display(),
        rules::config_path(root).display()
    )
}

/// Locate a repository's configuration, warning if it is in the old place.
pub fn locate_config(root: &Path) -> Option<Located> {
    let found = rules::locate(root)?;
    if found.is_legacy() {
        crate::output::warning(&legacy_config_notice(found.path(), root));
    }
    Some(found)
}

/// A repository's audit suppressions, from wherever they are written.
///
/// `[suppress]` in `openinvar.toml` is the supported source. A pre-0.3.0
/// `audit-ignore` file is still read when there is no `[suppress]` table, so
/// an upgrade does not quietly un-suppress a finding somebody deliberately
/// accepted — the audit equivalent of dropping a rule on upgrade.
pub fn load_suppressions(root: &Path) -> Result<Suppressions, String> {
    if let Some(found) = rules::locate(root) {
        let parsed = rules::load(found.path()).map_err(|e| e.to_string())?;
        if !parsed.suppress.is_empty() {
            return Ok(Suppressions::from_map(parsed.suppress));
        }
    }

    let legacy = openinvar_core::audit::legacy_ignore_path(root);
    if legacy.is_file() {
        crate::output::warning(&format!(
            "{} is deprecated and will stop being read in 1.0.0. Move these ids \
             into the [suppress] table of {}.",
            legacy.display(),
            rules::config_path(root).display()
        ));
        return Suppressions::load(&legacy);
    }
    Ok(Suppressions::default())
}
