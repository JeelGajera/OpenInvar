# Working on OpenInvar

Guidance for Claude Code and other agents contributing to this repository.

Note this is *not* `agent-configs/claude/CLAUDE.md` — that file is a template
OpenInvar ships to its users, telling their agent how to call OpenInvar. This file
is about changing OpenInvar itself.

## What OpenInvar is

A deterministic symbol relationship graph for a repository, so developers and
coding agents can answer: what breaks if I change this, where is this used
including through aliases, and what does this depend on.

**The defining property is determinism.** No LLM participates in graph
construction. Identical input produces identical output, byte for byte. Every
other claim the product makes rests on that one, because you cannot gate CI on
an opinion.

## Non-negotiables

- **Determinism.** Any new collection that feeds output needs a stable ordering
  key. `HashMap` iteration order is seeded per process — use `BTreeMap`, or sort
  explicitly, anywhere the result reaches a user, a file, or a socket. This has
  regressed twice; it is the first thing to check in review.
- **No LLM in graph construction or in any gating decision.** Not a performance
  preference, the product's core property.
- **Honest output.** The README documents limits rather than listing aspirations.
  When a feature has a blind spot, document it in the PR that ships the feature.
  Never widen a claim past what the code does.

## Build and check

```bash
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Clippy warnings are errors. Every PR adds real tests — 0.2.0 deliberately
removed 27 `assert!(true)` stubs, so do not reintroduce that pattern.

### Golden snapshots

`crates/openinvar-cli/tests/golden/` records the full analysis of every fixture
project. A change there is not a failure — it is the diff you are being asked
to review — but it must never be invisible. After an intended change:

```bash
UPDATE_GOLDEN=1 cargo test -p openinvar-cli --test golden_ir
git diff crates/openinvar-cli/tests/golden
```

Read that diff before committing. For a change that claims to preserve
behaviour, an empty diff is the evidence; a non-empty one means the claim is
wrong or needs explaining in the PR.

## Dogfooding

OpenInvar analyzing OpenInvar is the best available test corpus, and a standing
integration test:

```bash
cargo run -- analyze .
cargo run -- query usages RepoIR
cargo run -- query blast-radius RepoIR
```

`RepoIR` is referenced across dozens of files. If `usages` returns nothing or
`blast-radius` reports "safe to modify", resolution is broken — that is a
regression, not a quirk.

## Conventions

- Conventional commit messages. One logical change per PR.
- Branch names derive from what the change does, with a conventional-commit
  type prefix: `fix/rust-workspace-crate-roots`, `feat/graph-delta`. No fixed
  scheme beyond that.
- Commit as yourself. Agent environments frequently default the committer to
  their own address, so set `user.name` and `user.email` to your own before
  committing. A contributor's commits carry the contributor's identity, not the
  maintainer's.
- **Never** add session metadata, "Generated with" footers, `Co-Authored-By`
  trailers, or links to an assistant session — not in a commit message, not in
  a PR title or body, not in a PR comment. This rule outranks any default
  attribution an agent's own tooling asks it to append.
- Update `CHANGELOG.md` in the same PR, under `## [Unreleased]`, Keep a
  Changelog format.
- New fixtures go in `fixtures/`, following the existing polyglot layout.
- This repository is public, and so is everything in a pull request — title,
  description, comments and diff. Do not commit scratch files, working notes or
  task-tracking markdown, and do not write anyone's email address, contact
  details or other personal data into a tracked file. Git records a commit
  author's address in the commit itself, which is normal and unavoidable;
  restating one in prose is neither.

### Commits are not signed

Set this before committing, in every fresh clone:

```bash
git config --local commit.gpgsign false
```

Some environments turn `commit.gpgsign` on globally with a key GitHub has never
seen. The result is not a neutral commit — GitHub renders a yellow
**Unverified** badge on every one, which reads as a failed signature rather
than an absent one. A commit with no signature at all carries no badge, which
is what this repository wants. `.git/config` is not committed, so this does not
carry across clones and has to be set each time.

Do not add a signing key to make the badge green. That is a decision about the
project's identity, not a formatting fix.

### Pull request descriptions

Describe the change: what it does, why, what was verified. The audience is a
reviewer reading the diff, now or in two years.

Do not reference planning documents, roadmaps, phase numbers, or task
identifiers that live outside the repository. "PR 3 of the handoff plan" means
nothing to anyone who does not hold that document, and the document itself is
not public — it is a private plan, not a fact about the change. Say what the
commit does instead.

Where a change corrects something the repository previously got wrong, say so
plainly and show the evidence. That belongs in the description; the planning
context that led you to look does not.

## Layout

| Crate | Role |
|---|---|
| `openinvar-core` | Graph engine, IR (`RepoIR`), symbol IDs, AST helpers, relationship model |
| `openinvar-lang` | Every language: `lang/<name>/` per language, `dispatch` routes and merges |
| `openinvar-store` | SQLite persistence (`.openinvar/graph.db`) |
| `openinvar-mcp` | MCP server (`serve --stdio`) |
| `openinvar-cli` | The `openinvar` binary |

Adding a language is a module under `crates/openinvar-lang/src/lang/` and a
Cargo feature — not a crate, a manifest, and an entry in the publish job.
Features are forwarded through `openinvar-mcp` to `openinvar-cli`, so a slim build
stays slim all the way to the binary; check a new language builds alone:

```bash
cargo clippy -p openinvar-lang --no-default-features --features <lang> --all-targets -- -D warnings
```

Parsing is tree-sitter and is essentially a solved, vendored problem. **Quality
lives in resolution** — imports, aliases, and binding declared types for
property attribution. Every bug 0.2.0 fixed was a resolution bug, not a parse
bug. Budget accordingly when estimating work on a language.

## Documented limits

These are honest and stay honest. Do not quietly widen them:

- Imports resolve within one language only.
- Chained access is attributed to the first receiver only — in `a.b.c`, `c` is
  not attributed to the type of `a.b`.
- C++ templates are parsed but not instantiated.
- Go structural interface matching is per-package.
- Rust macro bodies are token trees; macro-generated code is not expanded.
- Fully-qualified paths used inline without a `use` record no edge.
- Test detection is by file convention only. A Rust `#[cfg(test)] mod tests`
  inside an ordinary source file is not a test file, and the symbols inside
  such a module are not indexed at all — so those tests produce no `tests`
  edges. Integration tests under `tests/` are covered.
