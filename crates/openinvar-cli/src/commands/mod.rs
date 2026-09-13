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

use std::path::{Path, PathBuf};

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
