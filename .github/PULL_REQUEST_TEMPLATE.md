<!--
Describe the change: what it does, why, and what you verified. The audience is
a reviewer reading the diff, now or in two years.

Everything here is public — title, description, comments and diff.
-->

## What this changes

<!-- The change itself. If it corrects something the repository previously got
     wrong, say so plainly and show the evidence. -->

## Why

<!-- The problem it solves. For a new rule kind or language, say what was not
     expressible or not resolvable before. -->

## Verification

<!-- Delete rows that do not apply; add anything else you ran. -->

| Check | Result |
|---|---|
| `cargo test --workspace` | |
| `cargo clippy --workspace --all-targets -- -D warnings` | |
| `UPDATE_GOLDEN=1` then `git diff` on golden | |
| `openinvar query usages RepoIR` | |

<!--
The golden diff is the important row. For a change that claims to preserve
behaviour, an empty diff is the evidence; a non-empty one belongs here with an
explanation of every entry.
-->

## Tests added

<!-- Which tests, and what property each one defends. If you broke the
     implementation on purpose to confirm they fail, say so. -->

## Limits

<!-- What this does not handle. A blind spot documented here is a feature; one
     a user finds later is a bug. If a limit is permanent, add it to the
     README's Known limits in this PR. -->

## Checklist

- [ ] `CHANGELOG.md` updated under `## [Unreleased]`
- [ ] New collections that reach output use `BTreeMap`/`BTreeSet` or sort explicitly
- [ ] No `assert!(true)`-style placeholder tests
- [ ] A new language builds on its own: `cargo clippy -p openinvar-lang --no-default-features --features <lang> --all-targets -- -D warnings`
- [ ] No scratch files, working notes, or personal data in the diff
