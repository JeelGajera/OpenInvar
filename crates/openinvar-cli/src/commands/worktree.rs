//! A throwaway checkout of one revision, for `analyze --at`.
//!
//! Analysing a past revision means having its files on disk. Checking them out
//! in place is not an option — it would destroy uncommitted work and leave the
//! repository on a detached HEAD if anything failed partway — so `--at` adds a
//! detached worktree in a temporary directory, analyses that, and removes it.
//!
//! **`git worktree` rather than `git archive`.** Piping `git archive` into tar
//! is lighter and touches no git metadata, but archive applies `export-ignore`
//! from `.gitattributes`: a repository that uses it would be analysed with
//! files silently absent, producing a graph missing symbols that are really
//! there. That is precisely the confidently wrong answer a gate must never
//! give, and it would be invisible — the analysis would look like a clean run.
//! A directory that has to be cleaned up is the cheaper problem.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A detached worktree at one revision, removed when this value is dropped.
///
/// Removal is in `Drop` rather than at the end of the happy path because every
/// error between creation and completion would otherwise leak both a directory
/// and an entry in the repository's `.git/worktrees`, and the entry is the part
/// a user would have to know about `git worktree prune` to clear.
pub struct TempWorktree {
    repo: PathBuf,
    path: PathBuf,
}

impl TempWorktree {
    /// Check `sha` out into a fresh temporary directory.
    ///
    /// `sha` is expected to be already resolved — `git worktree add` accepts a
    /// committish, but resolving first means an unknown revision is reported as
    /// such rather than as a worktree failure.
    pub fn add(repo: &Path, sha: &str) -> Result<Self, String> {
        // git requires the path not to exist yet. Process id and a
        // high-resolution timestamp keep concurrent runs on one machine apart;
        // the name never reaches the analysis, which records relative paths
        // only, so it cannot affect the result.
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "openinvar-at-{}-{unique}",
            std::process::id()
        ));

        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["worktree", "add", "--detach", "--quiet"])
            .arg(&path)
            .arg(sha)
            .output()
            .map_err(|err| format!("could not run git to check out '{sha}': {err}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "could not check out '{sha}' into a temporary worktree: {}",
                stderr.trim()
            ));
        }

        Ok(Self {
            repo: repo.to_path_buf(),
            path,
        })
    }

    /// The directory the revision is checked out in.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempWorktree {
    fn drop(&mut self) {
        let removed = Command::new("git")
            .arg("-C")
            .arg(&self.repo)
            .args(["worktree", "remove", "--force"])
            .arg(&self.path)
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false);

        if removed {
            return;
        }

        // Falling back matters: `worktree remove` refuses in cases the
        // directory itself is fine to delete, and leaving the registration
        // behind would make the *next* run fail on a path git still believes
        // is in use. `prune` clears the record once the directory is gone.
        let _ = std::fs::remove_dir_all(&self.path);
        let pruned = Command::new("git")
            .arg("-C")
            .arg(&self.repo)
            .args(["worktree", "prune"])
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false);

        if !pruned {
            // stderr, never stdout: `--json` must put a parseable document on
            // stdout and nothing else, and a drop can happen mid-serialisation.
            eprintln!(
                "warning: could not clean up the temporary worktree at {}. \
                 Run `git worktree prune` to clear it.",
                self.path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Resolves a revision in this repository, or skips when there is no git
    /// history — a source checkout without one is not a failure of this code.
    fn head_sha() -> Option<String> {
        let root = std::env::current_dir().ok()?;
        openinvar_store::revision::resolve(&root, "HEAD").ok()
    }

    #[test]
    fn a_worktree_carries_the_revisions_files() {
        let Some(sha) = head_sha() else { return };
        let root = std::env::current_dir().expect("cwd");

        let worktree = TempWorktree::add(&root, &sha).expect("worktree adds");
        assert!(worktree.path().is_dir(), "no directory was created");
        assert!(
            worktree.path().join("Cargo.toml").is_file(),
            "the checkout is missing a file that is certainly in this revision"
        );
    }

    #[test]
    fn dropping_it_removes_the_directory_and_the_registration() {
        let Some(sha) = head_sha() else { return };
        let root = std::env::current_dir().expect("cwd");

        let path = {
            let worktree = TempWorktree::add(&root, &sha).expect("worktree adds");
            worktree.path().to_path_buf()
        };

        assert!(!path.exists(), "the worktree directory outlived the guard");

        // The registration matters as much as the directory: a leaked entry
        // makes a later `worktree add` fail on a path git thinks is in use.
        let listed = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["worktree", "list"])
            .output()
            .expect("git worktree list");
        let listed = String::from_utf8_lossy(&listed.stdout);
        assert!(
            !listed.contains(&*path.to_string_lossy()),
            "git still lists the removed worktree:\n{listed}"
        );
    }

    #[test]
    fn an_unresolvable_revision_fails_rather_than_checking_out_something_else() {
        let root = std::env::current_dir().expect("cwd");
        let result = TempWorktree::add(&root, "definitely-not-a-revision-xyz");
        assert!(
            result.is_err(),
            "a bad revision produced a worktree: {:?}",
            result.map(|w| w.path().to_path_buf())
        );
    }

    #[test]
    fn two_worktrees_of_one_revision_do_not_collide() {
        let Some(sha) = head_sha() else { return };
        let root = std::env::current_dir().expect("cwd");

        let first = TempWorktree::add(&root, &sha).expect("first worktree");
        let second = TempWorktree::add(&root, &sha).expect("second worktree");
        assert_ne!(
            first.path(),
            second.path(),
            "two checkouts of one revision landed on the same path"
        );
    }
}
