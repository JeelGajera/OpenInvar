# Contributing to OpenInvar

Thanks for considering it. This document is the on-ramp: what the project
guarantees, how to build and check your work, and what a mergeable change looks
like.

If you are looking for how to *use* OpenInvar, that is [README.md](README.md).
If you are an AI agent working on this repository, read
[CLAUDE.md](CLAUDE.md) as well — it carries the same rules in the form an agent
needs them.

## The one property everything else rests on

**OpenInvar is deterministic. No LLM participates in graph construction, or in
any gating decision.** Identical input produces identical output, byte for byte.

This is not a performance preference. It is the reason the tool is allowed to
fail a build: a check that returns a different answer on a re-run is a check
nobody can act on. Every feature, every optimisation and every dependency is
subordinate to it.

Two rules follow directly, and reviewers will hold you to both:

- **Ordering is explicit.** `HashMap` and `HashSet` iterate in an order seeded
  per process. Anywhere a result reaches a user, a file, a socket or a
  snapshot, use `BTreeMap`/`BTreeSet` or sort explicitly. This has regressed
  twice in the project's history, and it is the first thing checked in review.
- **Honest output.** When a feature has a blind spot, document it in the pull
  request that ships the feature and in the README's
  [Known limits](README.md#known-limits). Never widen a claim past what the
  code actually does. A limit written down is a feature; a limit discovered by
  a user is a bug.

## Getting started

```bash
git clone https://github.com/JeelGajera/OpenInvar
cd OpenInvar
cargo build --release
cargo test --workspace
```

There is nothing to set in the environment and no system libraries to install.
The store is SQLite, compiled from the bundled amalgamation — one C file. A CI
job builds with the newest GCC the runner offers and with no `CC`/`CXX` set at
all, specifically so that stays true. If you hit a toolchain error in a
dependency, that is a bug worth reporting rather than something to work around
locally.

The toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml), so
`rustup` will fetch the right compiler on first build.

## Run what CI runs

CI is not a surprise. Every check it performs, you can run first:

```bash
# the workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# determinism: two analyses of identical input must be byte-identical
cargo build -p openinvar-cli
./target/debug/openinvar analyze . --json > /tmp/a1.json
./target/debug/openinvar analyze . --json > /tmp/a2.json
cmp /tmp/a1.json /tmp/a2.json && echo reproducible

# each language must also build alone (see "Adding a language")
cargo clippy -p openinvar-lang --no-default-features --features <lang> \
  --all-targets -- -D warnings
```

**Clippy warnings are errors.** There is no warning budget.

### Golden snapshots

[`crates/openinvar-cli/tests/golden/`](crates/openinvar-cli/tests/golden)
records the full analysis of every fixture project. A change there is not a
failure — it is the diff you are asking a reviewer to approve — but it must
never be invisible:

```bash
UPDATE_GOLDEN=1 cargo test -p openinvar-cli --test golden_ir
git diff crates/openinvar-cli/tests/golden
```

Read that diff before you commit, and explain it in the pull request. For a
change that claims to preserve behaviour, **an empty diff is the evidence**; a
non-empty one means the claim is wrong or needs explaining.

### Dogfooding

OpenInvar analysing OpenInvar is the best test corpus available, and a standing
check:

```bash
cargo run -- analyze .
cargo run -- query usages RepoIR
cargo run -- query blast-radius RepoIR
```

`RepoIR` is referenced across dozens of files. If `usages` returns nothing, or
`blast-radius` reports "safe to modify", resolution is broken. That is a
regression, not a quirk.

This repository also gates on its own binary — see
[`openinvar.toml`](openinvar.toml) and
[`.github/workflows/openinvar.yml`](.github/workflows/openinvar.yml). Your
change has to keep that passing:

```bash
cargo run -- check . --require-rules
```

## Project layout

| Crate | Role |
|---|---|
| [`openinvar-core`](crates/openinvar-core) | Graph engine, IR (`RepoIR`), symbol IDs, AST helpers, relationship model, rule schema and evaluation |
| [`openinvar-lang`](crates/openinvar-lang) | Every language: `lang/<name>/` per language, `dispatch` routes and merges |
| [`openinvar-store`](crates/openinvar-store) | SQLite persistence (`.openinvar/graph.db`) |
| [`openinvar-mcp`](crates/openinvar-mcp) | MCP server (`serve --stdio`) |
| [`openinvar-cli`](crates/openinvar-cli) | The `openinvar` binary |

Dependencies run in one direction only, and the repository enforces it on
itself with a `layers` rule: `cli` → `mcp` → `store` → `lang` → `core`. The
engine depends on nothing else in the workspace. If you find yourself wanting
`openinvar-core` to call into `openinvar-lang`, that is the design pushing
back — see how `requires-test` solved exactly that problem without the
dependency.

## Adding a language

A language is **a module and a Cargo feature** — not a crate, a manifest and an
entry in the publish job.

1. Add `crates/openinvar-lang/src/lang/<name>/`, following an existing
   language. [`rust`](crates/openinvar-lang/src/lang/rust) and
   [`typescript`](crates/openinvar-lang/src/lang/typescript) are the most
   complete; [`csharp`](crates/openinvar-lang/src/lang/csharp) is the smallest
   Tier 1 example, and [`ruby`](crates/openinvar-lang/src/lang/ruby) the only
   Tier 2 one.
2. Add the feature to
   [`crates/openinvar-lang/Cargo.toml`](crates/openinvar-lang/Cargo.toml),
   pulling in only that language's grammar, and list it under `tier1` or
   `full`.
3. Route it in `dispatch`.
4. Add a fixture under [`fixtures/`](fixtures), following the existing
   polyglot layout.
5. Check it builds **alone**:

```bash
cargo clippy -p openinvar-lang --no-default-features --features <name> \
  --all-targets -- -D warnings
```

That last step is not ceremony. Code that compiles only because some *other*
language pulled a name into scope compiles nowhere else, and a slim binary is
what a user gets when they build for one language. CI runs this for every
feature the crate declares, so a new language is covered without anyone
remembering to list it twice.

### Tier 1 vs Tier 2

**Tier 1** languages have their imports, aliases and declared types resolved.
Gates act on them.

**Tier 2** languages are analysed through the structural analyzer using the
grammar's own `tags.scm`: symbols are found, but matching is by name within one
file. **A gate never draws a conclusion from a structural region** — rules
report `Inconclusive` there instead. Shipping a Tier 2 language costs a user
nothing and gives them symbol lookup; claiming it is Tier 1 costs them a
false pass.

Parsing is tree-sitter and is essentially a solved, vendored problem.
**Quality lives in resolution** — imports, aliases, and binding declared types
for property attribution. Every bug 0.2.0 fixed was a resolution bug, not a
parse bug. Budget accordingly.

## Adding a rule kind

Rules live in two files:
[`rules.rs`](crates/openinvar-core/src/rules.rs) is the schema and its
validation; [`rule_eval.rs`](crates/openinvar-core/src/rule_eval.rs) is the
evaluation. Add a variant to `RuleKind`, parse and validate it at load time,
give it a `default_severity`, write its evaluator, and add a test file under
[`crates/openinvar-core/tests/`](crates/openinvar-core/tests).

The hard part is not finding violations. It is being honest about what you
**cannot** see.

### The four verdicts

| Verdict | Meaning |
|---|---|
| `Satisfied` | The rule holds, and the evidence was strong enough to say so |
| `Violated` | A specific, nameable breach — **the only verdict that fails a gate** |
| `Inconclusive` | Evidence in scope was too weak to rule a violation out |
| `Skipped` | The rule needs a diff and none was supplied |

`Inconclusive` is the verdict that makes the tool trustworthy, and getting it
right is most of the work in a new kind. The discipline:

- **An in-scope edge that is not `gate_safe` counts as uncertainty whether or
  not its recorded target matches.** A weak edge could point anywhere, so it
  could always have been the one that mattered.
- **Ask which direction uncertainty runs.** It is not symmetric. For
  `forbid-dependency` an unresolved edge might be a *violation*; for
  `requires-dependency` the very same edge might be the one that *satisfies*
  the rule. For `layers`, the outermost layer has nothing above it, so its
  unresolved edges cannot be upward violations at all and must not be counted.
- **A found violation still reports** even when other evidence is weak. Weak
  evidence elsewhere does not make a real cycle less of a cycle.
- **Absence claims are the hard class.** A missing edge and an unresolved edge
  look identical. If your rule asserts that something is *not* there, assume
  `Inconclusive` is the common case and justify any `Satisfied`.

### Pick the severity deliberately

`default_severity` splits by category: stated boundaries default to `error`,
hygiene rules (`max-fan-in`, `max-fan-out`, `no-orphans`) default to `warn`.
A rule that fires on a healthy repository on day one is a rule people switch
off, and a switched-off rule protects nothing.

## Tests

**Every pull request adds real tests.** Version 0.2.0 deliberately removed 27
`assert!(true)` stubs; please do not reintroduce that pattern.

A test earns its place by failing when the code is wrong. If you are unsure
yours does, break the implementation on purpose and confirm the test catches
it — that takes a minute and is the difference between a test and a comment.

Prefer tests that pin the *reasoning*, not just the output. The rule tests in
`crates/openinvar-core/tests/` are named after the property they defend
(`an_unresolved_edge_might_be_the_satisfying_one`,
`the_top_layer_is_never_uncertain`), which is why a later change that breaks
the reasoning is legible in the failure list.

## Commits and pull requests

- **Conventional commit messages.** `feat(check): add no-cycles`,
  `fix(rust): resolve aliased re-exports`.
- **One logical change per pull request.**
- **Branch names derive from what the change does**, with a
  conventional-commit type prefix: `fix/rust-workspace-crate-roots`,
  `feat/graph-delta`. No fixed scheme beyond that.
- **Update [`CHANGELOG.md`](CHANGELOG.md)** in the same pull request, under
  `## [Unreleased]`, in [Keep a
  Changelog](https://keepachangelog.com/en/1.1.0/) format.
- **Describe the change in the pull request**: what it does, why, and what you
  verified. The audience is a reviewer reading the diff, now or in two years.
  Where a change corrects something the repository previously got wrong, say so
  plainly and show the evidence.
- **This repository is public, and so is everything in a pull request** —
  title, description, comments and diff. Please do not commit scratch files,
  working notes or task-tracking markdown, and do not write anyone's email
  address, contact details or other personal data into a tracked file. Git
  records a commit author's address in the commit itself, which is normal and
  unavoidable; restating one in prose is neither.

A pull request is easiest to merge when it says which of the checks above you
ran and what they returned — particularly the golden diff and the dogfooding
numbers.

## Reporting bugs

Open an [issue](https://github.com/JeelGajera/OpenInvar/issues). The bug report
template asks for the version, the language, and a snippet that reproduces the
behaviour, because resolution bugs are almost always specific to a construct.

The most useful bug report in this project is *"OpenInvar said X about this
code and X is wrong"*, with the code. A missing edge, a wrong attribution or a
gate that passed when it should not have are all worth reporting, as is
anything non-deterministic — two runs that disagree is the most serious class of
bug here, whatever else it looks like.

**Do not open a public issue for a security vulnerability.** See
[SECURITY.md](SECURITY.md).

## Conduct

By participating you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).

## Licence

OpenInvar is Apache-2.0. Contributions are accepted under the same licence —
see [LICENSE](LICENSE). There is no CLA.
