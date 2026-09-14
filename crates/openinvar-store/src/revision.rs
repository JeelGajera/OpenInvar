//! Naming the revision a snapshot is recorded under.
//!
//! Lives beside the store because the store keys snapshots by these names, and
//! because more than one front end resolves them: the CLI for `--snapshot`,
//! `diff` and `check`, and the MCP server for the equivalent tools. Two copies
//! of this rule would drift, and the failure mode when they do is a snapshot
//! written under one name and looked for under another.
//!
//! `diff` compares two graphs, so each has to be stored under a name that
//! identifies what was analysed. Three forms are accepted, and the distinction
//! between them is not cosmetic:
//!
//! - `HEAD`, or any other committish, resolves through git to the commit it
//!   names. Storing the resolved SHA rather than the label is what makes the
//!   snapshot still mean something after the branch moves.
//! - A SHA is used as given, once git confirms it exists.
//! - `worktree` is the working tree including uncommitted edits. It has no
//!   commit to resolve to, so it keeps that literal name and is expected to be
//!   overwritten constantly.

use std::path::Path;
use std::process::Command;

/// The literal name for "the working tree as it is right now".
pub const WORKTREE: &str = "worktree";

/// Resolve a `--snapshot` argument to the name its graph is stored under.
///
/// Errors rather than falling back to the raw string when git cannot resolve
/// a revision. A typo silently becoming its own snapshot name would produce a
/// `diff` against an empty graph, which reads as "everything was added" — a
/// confidently wrong answer where an error is the honest one.
pub fn resolve(root: &Path, revision: &str) -> Result<String, String> {
    if revision == WORKTREE {
        return Ok(WORKTREE.to_string());
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .arg("--verify")
        .arg("--quiet")
        .arg(format!("{revision}^{{commit}}"))
        .output()
        .map_err(|err| format!("could not run git to resolve '{revision}': {err}"))?;

    if !output.status.success() {
        return Err(format!(
            "'{revision}' is not a revision in this repository. \
             Pass a commit, a branch, a tag, or '{WORKTREE}' for uncommitted work."
        ));
    }

    let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if sha.is_empty() {
        return Err(format!("git resolved '{revision}' to nothing"));
    }
    Ok(sha)
}

/// Resolve a revision that has to name a commit, rejecting `worktree`.
///
/// `analyze --at` reads a revision's tree out of git, and the working tree is
/// not something git can check out — it is what you already have. Accepting the
/// name and quietly analysing the working tree instead would record a
/// working-tree graph under a name every reader takes for a commit, which is
/// the kind of confidently mislabelled result a later `diff` cannot detect.
///
/// Separate from [`resolve`] rather than a flag on it because the two have
/// different vocabularies: `--snapshot` offers three forms and this offers two,
/// and an error message that lists an option the caller will then refuse is
/// how a user ends up trying `worktree` twice.
pub fn resolve_commit(root: &Path, revision: &str) -> Result<String, String> {
    if revision == WORKTREE {
        return Err(format!(
            "'{WORKTREE}' is the working tree, which is what `analyze` reads by default. \
             Pass a commit, a branch, or a tag."
        ));
    }

    resolve(root, revision).map_err(|err| {
        // Only the "unknown revision" case names the accepted forms, so only
        // that one needs rewording; anything else (git missing, git failing) is
        // reported as it happened.
        if err.contains(WORKTREE) {
            format!(
                "'{revision}' is not a revision in this repository. \
                 Pass a commit, a branch, or a tag."
            )
        } else {
            err
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_needs_no_git_and_keeps_its_name() {
        // Deliberately a path that is not a repository: the working tree name
        // must resolve without consulting git at all, since it names something
        // git has no commit for.
        let resolved = resolve(Path::new("/nonexistent"), WORKTREE).expect("worktree resolves");
        assert_eq!(resolved, WORKTREE);
    }

    #[test]
    fn an_unknown_revision_is_an_error_not_a_name() {
        let root = std::env::current_dir().expect("cwd");
        let result = resolve(&root, "definitely-not-a-revision-xyz");
        assert!(
            result.is_err(),
            "an unresolvable revision became a snapshot name: {result:?}"
        );
    }

    #[test]
    fn resolve_commit_refuses_the_working_tree() {
        let root = std::env::current_dir().expect("cwd");
        let err = resolve_commit(&root, WORKTREE).expect_err("worktree is not a commit");
        assert!(
            err.contains("commit"),
            "the refusal does not say what to pass instead: {err}"
        );
    }

    #[test]
    fn resolve_commit_does_not_offer_worktree_when_a_revision_is_unknown() {
        // The shared resolver lists `worktree` among the accepted forms, which
        // is right for `--snapshot` and wrong here: a user told to try it would
        // have it refused by the check above.
        let root = std::env::current_dir().expect("cwd");
        let err = resolve_commit(&root, "definitely-not-a-revision-xyz")
            .expect_err("unknown revision is an error");
        assert!(
            !err.contains(WORKTREE),
            "the error offers an option this function rejects: {err}"
        );
    }

    #[test]
    fn resolve_commit_agrees_with_resolve_on_a_real_commit() {
        let root = std::env::current_dir().expect("cwd");
        let (Ok(plain), Ok(commit)) = (resolve(&root, "HEAD"), resolve_commit(&root, "HEAD")) else {
            // A checkout without git history is not a failure of this code.
            return;
        };
        assert_eq!(
            plain, commit,
            "the two resolvers disagree about what HEAD is"
        );
    }

    #[test]
    fn head_resolves_to_a_full_sha() {
        // Runs inside this repository, so HEAD exists. Storing the SHA rather
        // than the label is the point: `HEAD` means something different
        // tomorrow, the SHA does not.
        let root = std::env::current_dir().expect("cwd");
        let Ok(resolved) = resolve(&root, "HEAD") else {
            // A checkout without git history is not a failure of this code.
            return;
        };
        assert_eq!(resolved.len(), 40, "expected a full SHA, got '{resolved}'");
        assert!(resolved.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(resolved, "HEAD", "the label was stored instead of the commit");
    }
}
