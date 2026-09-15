# OpenInvar

**Deterministic integrity checks for AI-written code.**
Catches structural violations, test tampering, and eroded contracts — at the
edit, at commit, or in CI. No model in the loop.

An agent opens a pull request. It compiles, the suite is green, and one of the
tests no longer asserts what it used to. OpenInvar reads the change as a graph
of symbol relationships and reports the specific thing that moved:

- **`openinvar audit`** — a test that stopped covering a symbol the same diff
  changed; a symbol removed while a caller still refers to it; code that only
  its own tests reach.
- **`openinvar check`** — the architectural constraints this repository wrote
  down, enforced across every language in it, exiting non-zero when one breaks.

No model participates in either. Identical input produces identical output,
byte for byte — which is the whole reason a gate can act on the result.

**Works alongside CodeGraph and GitNexus.** Those answer *what is this code*;
OpenInvar answers *did this change break something it was not supposed to*. If
one of them is already indexing your repository, keep it — nothing here
competes for that job.

## Why OpenInvar

- **Deterministic: no LLM participates in graph construction, or in any gating
  decision.** Identical input produces identical output, byte for byte. This is
  first because everything else depends on it — *you cannot gate CI on an
  opinion*. A tool that answers "what breaks if I change this" probabilistically
  can advise, but it cannot block a merge, because a check that returns a
  different answer on a re-run is a check nobody can act on. Every competing
  tool is probabilistic somewhere in that path.
- **Honest about what it does not know.** Coverage is published, per language,
  on every run; gates fail open on regions OpenInvar cannot resolve rather than
  reporting a pass they have not earned. See [Resolution
  coverage](#resolution-coverage).
- Alias-aware: resolves `import { A as B }`, so a rename finds the callers a
  text search misses.
- Property-aware: attributes `payload.user_id` to the type the value was
  declared as.
- Fast queries: in-memory graph traversal.
- Agent-ready: hooks, an MCP server, and a GitHub Action — push as well as pull.

### Resolution coverage

An enforcement tool that tells you what it could **not** resolve is worth more
than one implying completeness, so `openinvar status` publishes the figure on
every run — overall and per language.

Measured on this repository, with the released binary:

```
Resolved           99.8% (5974 of 5986 edge(s))
  C#               100.0% of 12 edge(s)
  C/C++            100.0% of 32 edge(s)
  Go               100.0% of 40 edge(s)
  Java             100.0% of 14 edge(s)
  Python           100.0% of 71 edge(s)
  Ruby             0.0% of 12 edge(s)
  Rust             100.0% of 5729 edge(s)
  TypeScript       100.0% of 76 edge(s)
  12 edge(s) are structural: matched by name within one file.
  A gate must not act on them.
```

The 0.0% row is a Tier 2 language, and it is the point: those twelve edges are
named rather than absorbed into a headline. C#'s row read `0.0% of 2 edge(s)`
until it was promoted, which is what a tier change looks like from here.

### Reference coverage, which is the stricter number

The figure above answers "can a gate act on what is in the graph". It does not
answer "how much of the source is in the graph", and on its own it reads as
though it does. An edge exists only once something bound it, so a reference the
analysis could not place leaves nothing to count — it never enters the
denominator, and failing to bind *more* references makes the percentage go
**up**.

So `status` reports both:

```
Resolved           99.8% (5980 of 5992 edge(s))
References bound   69.6% (5980 of 8597 reference(s))
  2617 reference(s) named in source bound to nothing in the graph.
```

Measured on this repository. The second number is the one that says how much of
the code OpenInvar actually describes, and publishing it is the point: a tool
that gates CI should not be the only party who knows where its blind spots are.

**An unbound reference is not automatically a defect.** A type from a package
whose source was never analysed is indistinguishable, to an adapter, from one it
should have found and missed — both are references the graph cannot answer
questions about, which is what the number is for. Read it as an upper bound on
what is absent. Rust's share is large here largely because prelude names, macro
bodies and fully-qualified paths used without a `use` are all documented limits
below.

It is comparable across revisions of one repository and only loosely between
languages: each extractor creates a different number of placeholders for the
same code, so finer-grained extraction reports a lower percentage for identical
source.

Measured across the evaluation corpus — four ordinary repositories nobody here
wrote — it lands in a narrow band:

| Repository | Language | References bound | Edge-level figure |
|---|---|---:|---:|
| rust-lang/log | Rust | 50.8% | 100.0% |
| pallets/click | Python | 62.3% | 100.0% |
| google/gson | Java | 64.2% | 100.0% |
| spf13/cobra | Go | 65.9% | 100.0% |

The right-hand column is why the left one exists. `log` is the low scorer and
the reason is visible: it is a 23-file crate made largely of `impl` blocks for
standard traits, and every unresolved type it reports by name is one of them —
`From`, `Display`, `PartialEq`, `Formatter`. Those are references to code
outside the repository, not things the analysis fumbled.

Full run in [`eval/results/`](eval/results), including where the figure fell
short of its own pre-registered threshold.

There is one released binary and it carries every language, so this is the
figure your binary reports. A build narrowed to fewer languages reports a
*higher* number on the same repository — not because it resolves more, but
because it never looks at the files it cannot carry. See [Building for one
language](#building-for-one-language).

## Install

macOS / Linux:
```bash
curl -fsSL https://raw.githubusercontent.com/JeelGajera/OpenInvar/master/install.sh | bash
```

Windows (PowerShell):
```powershell
irm https://raw.githubusercontent.com/JeelGajera/OpenInvar/master/install.ps1 | iex
```

Cargo (crates.io):
```bash
cargo install openinvar-cli
```

From source:
```bash
cargo install openinvar-cli --git https://github.com/JeelGajera/OpenInvar
```

## Quick Start

```bash
# 1. record the graph as it stands before the change
openinvar analyze . --snapshot HEAD

# 2. let the agent work, then ask what the change actually did
openinvar audit . --base HEAD --head worktree

# 3. enforce the rules this repository wrote down
openinvar check . --diff-only
```

`audit` exits `1` when it finds something at the requested severity, `check`
exits `1` on a broken rule — so either is a CI gate with no further wiring. Both
report what they could *not* decide rather than passing quietly.

## Commands

The two that gate:

| Command | Purpose |
|---|---|
| `openinvar audit [--base <rev>] [--head <rev>]` | Detect changes made to pass a check rather than to work |
| `openinvar check [--diff-only] [--require-rules]` | Enforce `openinvar.toml`; exit 1 on violation |

Everything they are built on:

| Command | Purpose |
|---|---|
| `openinvar analyze <path> [--snapshot <rev>] [--at <rev>] [--json]` | Build the graph into `.openinvar/graph.db`, optionally recording it |
| `openinvar diff --base <rev> --head <rev>` | What changed between two recorded revisions |
| `openinvar tests <symbol> \| --diff` | Which tests exercise a symbol or a change |
| `openinvar report --base <rev> --head <rev>` | One markdown report for a PR comment |
| `openinvar status` | Graph stats, per-language tier, resolution coverage |
| `openinvar watch <path>` | Keep the graph in sync on file changes |
| `openinvar serve --stdio` | MCP server |

And the graph queries underneath, covered in [It also answers graph
questions](#it-also-answers-graph-questions): `openinvar query`,
`openinvar impact`, `openinvar context`.

### Analyzing a past revision

`--snapshot` records whatever is on disk. `--at` goes and gets a revision:

```bash
# record the graph for a commit nobody has checked out
openinvar analyze . --at 4f2a1c9

# then compare it to anything else already recorded
openinvar diff . --base 4f2a1c9 --head worktree
```

Useful for building history without a checkout per commit — walking a range to
see when a dependency first appeared, or comparing a release tag to `HEAD`.

Three things it will not do:

- **Touch your working tree.** The revision is checked out into a temporary
  worktree and removed afterwards. Uncommitted work is untouched and `HEAD`
  does not move.
- **Replace the working graph.** `query`, `check` and `audit` read the working
  graph when nobody names a revision, so `--at` records its revision and leaves
  that alone. A command answering about a past revision without saying so is
  the failure this avoids.
- **Accept `worktree`.** That names the tree you already have, which is what
  plain `analyze` reads. `--at` takes a commit, branch or tag, and pairs with
  `--snapshot` only by replacing it — the revision named is the one recorded.

Two runs of `--at` on one revision produce byte-identical output, temporary
directory notwithstanding: the checkout's path never enters the result.

## Audit

```bash
openinvar audit --base HEAD --head worktree
```

Detects changes that look like they were made to pass a check rather than to
work. Deterministic — no model is involved, here or anywhere else.

| Detector | Signal | Severity |
|---|---|---|
| `test-tampering` | A test stopped covering a symbol that changed in the same diff | error |
| `assertion-removal` | A test kept covering a symbol that changed, but lost assertions | error |
| `contract-erosion` | A symbol was removed while a surviving caller still referred to it | error |
| `dead-on-arrival` | A new symbol only tests refer to | warn |

`test-tampering` catches coverage disappearing. `assertion-removal` catches the
subtler move: the test still runs, still references the symbol, and no longer
checks anything — the `assert_eq!` became a call with its result dropped, and
both the suite and the coverage graph stayed green.

Assertions are counted during the same parse the symbols come from, so there is
no second source of truth to disagree with the graph. Only unambiguous forms
count — Rust's `assert*` macros, Python's `assert` and the `unittest`
`assertX` family, `expect`/`assert` in TypeScript and JavaScript, testify plus
`t.Error`/`t.Fatal` in Go, and `assert` in C and C++. Not `unwrap()`, which is
ordinary code as often as it is a check.

A count only means something where the language is counted, and where **both**
revisions counted it. Elsewhere the number is unknown rather than zero, and the
detector declines to conclude — which is also what happens against a snapshot
recorded before assertion counts existed.

An audit finding is an accusation, so the output is built to be checked rather
than believed. Every finding carries its evidence and the id you would write
down to suppress it. Detectors that did **not** run are named with the reason,
because an absent check otherwise reads as a passing one — and only files a
Tier 1 adapter resolved are in scope, since a name matched inside one file
cannot support an accusation.

Record a deliberate exception in the `[suppress]` table of `openinvar.toml`:

```toml
[suppress]
"test-tampering-1a2b3c4d" = "the test was rewritten when the API changed"
```

In the committed file on purpose. Suppressing a finding is an act somebody
should be able to review in a diff.

Suppressed findings are still shown, and a suppression that matches nothing is
reported as stale. Exit 0 when nothing was found at the requested severity, 1
when something was, 2 when the audit could not run.

## Check

A repository states its own constraints in `openinvar.toml`, and
`openinvar check` enforces them — across every language in the repository, which
is what no single-language architecture linter can do.

```
openinvar.toml      # rules and audit suppressions — commit this
.openinvar/
  graph.db          # generated — .gitignore this
```

The rules file sits at the root and is **meant to be committed**: a constraint
nobody can read is not a constraint. Only the generated graph belongs in
`.openinvar/`, so `.gitignore` needs one line:

```gitignore
.openinvar/
```

```toml
[[rule]]
name = "core-must-not-depend-on-cli"
kind = "forbid-dependency"
from = "crates/openinvar-core/**"
to   = "crates/openinvar-cli/**"
severity = "error"          # error (default) or warn

[[rule]]
name = "payload-is-stable"
kind = "no-field-removal"
symbol = "UserPayload"

[[rule]]
name = "god-node"
kind = "max-fan-in"
threshold = 60
severity = "warn"

[[rule]]
name = "architecture"
kind = "layers"
layers = ["src/api/**", "src/service/**", "src/core/**"]   # outermost first

[[rule]]
name = "features-do-not-talk"
kind = "independence"
modules = ["src/billing/**", "src/search/**", "src/inbox/**"]

[[rule]]
name = "no-import-cycles"
kind = "no-cycles"
scope = "src/**"      # optional; the whole graph by default
level = "file"        # file (default) or module

[[rule]]
name = "new-api-is-covered"
kind = "requires-test"
symbols  = "src/api/**"
new_only = true       # judge only what this change added
```

`requires-test` is the rule no comparable tool can express: `tests` edges are
derived across every Tier 1 language here, and semgrep, dependency-cruiser,
ArchUnit and import-linter have no equivalent in any language.

Coverage counts **direct** edges only. That a test reaches `login`, and `login`
references `format_token`, is no proof the test ever executes `format_token` —
it may sit behind an early return. It is also the difference between a rule and
a loophole: wire a new function into any legacy endpoint that already has a
test, and a transitive version goes green without one assertion being written.

`new_only = true` is what makes it adoptable. *Every symbol you added is
covered* is enforceable on a repository that could never pass *every symbol is
covered*, which is a coverage project rather than a gate.

`requires-dependency` inverts `forbid-dependency`, and the inversion matters:
there an unresolved edge might be a *violation*, here it might be the edge that
*satisfies* the rule. Only a symbol with no qualifying edge and nothing
unresolved is reported.

`no-orphans` is the weakest rule here and defaults to `warn`. Orphanhood is
pure absence, so any edge that did not resolve could be the reference that
makes a symbol reachable — on a repository with structural regions it is
undecided nearly always. `roots` exists because an entry point is unreferenced
by definition, and a rule that reports every `main` is one nobody keeps on.

`layers` states a dependency direction once instead of as every forbidden
pair — a five-layer stack is ten `forbid-dependency` rules written by hand. A
layer may depend downward and on itself; only reaching up is a violation, and
the message names both layers and their positions:

```
crates/openinvar-lang/src/dispatch.rs (layer 3: 'crates/openinvar-lang/**')
depends upward on crates/openinvar-store/src/sqlite.rs
(layer 2: 'crates/openinvar-store/**') via imports
```

The outermost layer may depend on everything, so an edge OpenInvar could not
resolve out of *that* layer is not counted as uncertainty — there is nothing
above it to violate. Every other layer's unresolved edges are, because they
could be pointing anywhere. `independence` has no permitted direction, so
every module's unresolved edges count.

| Kind | Fields | Fires when |
|---|---|---|
| `layers` | `layers` | A layer depends on one above it in the stack |
| `independence` | `modules` | Two modules declared independent reference each other, either direction |
| `forbid-dependency` | `from`, `to` | An import or re-export runs from one path glob to another |
| `forbid-reference` | `from`, `to` | *Any* reference does — the superset, so "you may call into this but not import it" is expressible |
| `no-field-removal` | `symbol` | A named symbol loses a field. Needs a change, so pass `--base`/`--head` |
| `max-fan-in` | `threshold` | A symbol exceeds that many inbound references. Third-party packages are not counted |
| `no-cycles` | `scope`, `level` | A dependency cycle exists within the scope |
| `requires-test` | `symbols`, `only`, `new_only` | A symbol in scope is reached by no test |
| `requires-dependency` | `from`, `to` | A symbol in `from` references nothing in `to` |
| `no-orphans` | `scope`, `roots` | A symbol in scope is referenced by nothing |
| `max-fan-out` | `threshold` | A symbol reaches out to more than that many **distinct** symbols |
| `naming-convention` | `symbols`, `matches`, `only` | A symbol in scope has a name the pattern does not match |

`severity` defaults by kind. A stated boundary — `layers`, `independence`, the
`forbid-*` pair, `no-field-removal`, `naming-convention` — defaults to `error`,
because a rule written without one is a rule someone means to enforce. The fan
limits default to `warn`: a high fan-in or fan-out is frequently intentional (a
utility module, an IR type, an entry point), and blocking CI on one stops a
developer who added a module before wiring up its callers. Write
`severity = "error"` to make those block too.

`max-fan-out` counts **distinct targets**, where `max-fan-in` counts edges. A
file that imports the same module on three lines reaches one thing, not three.

`naming-convention` is the only kind that can never be undecided: names come
from the parse rather than from resolution, so even a file whose imports
resolve to nothing reports the names it declares.

Everything that can be wrong is wrong at parse time — an unknown kind, a glob
that does not compile, a missing field, a threshold of zero, two rules under one
name. A typo would otherwise sit in a repository until the day it was supposed
to catch something, and a gate that quietly enforces four of your five rules
reports a pass it has not earned.

```bash
openinvar check .                # rules that need only the current graph
openinvar check . --diff-only    # also the ones that need a change
```

**A rule has three outcomes, not two.** `forbid-dependency` is a claim about
*absence*, so reporting it satisfied claims every edge in scope was examined.
Where weaker edges could hide a violation the verdict is **undecided** — never
a pass, and never a failure either. That is how a Tier 2 region fails open
instead of passing quietly. Exit `0` clean, `1` a rule broken on resolved
evidence, `2` the check could not run at all.

## Test impact

```bash
openinvar tests UserPayload
openinvar tests --diff --base HEAD --head worktree
```

Returns the tests that exercise a symbol, or everything a change touched, so a
verify loop can run those instead of the whole suite or nothing at all.

This answer licenses an omission — naming a subset is a claim that the tests
left out cannot fail — so the confidence is carried by the exit status rather
than only printed:

| Exit | Means |
|---|---|
| `0` | Every changed symbol is reached by a resolved test edge; run this subset |
| `3` | Tests were found, but something could be missing; run the full suite |
| `2` | The question could not be answered |

A selection is incomplete when a changed symbol is reached by no recognized
test, when a test was selected on structural evidence, or when the repository
has structural regions at all — a test in one of those could exercise the
change without recording an edge.

Tests the change itself modified are reported separately and never counted as
coverage. A diff that edits a function and its only test is exactly where a
reviewer most needs to be told something is missing.

## Agent Hooks

MCP is pull-only: the agent has to decide to ask. Hooks are push — the graph
reaches the agent at the moment of the edit.

```bash
mkdir -p .claude/hooks
cp agent-configs/hooks/claude/*.sh .claude/hooks/
cp agent-configs/hooks/lib/openinvar-hook-lib.sh .claude/hooks/
chmod +x .claude/hooks/*.sh
# then merge agent-configs/hooks/claude/settings.json into .claude/settings.json

cp agent-configs/hooks/git/pre-commit  .git/hooks/pre-commit
cp agent-configs/hooks/git/post-commit .git/hooks/post-commit
cp agent-configs/hooks/lib/openinvar-hook-lib.sh .git/hooks/
chmod +x .git/hooks/pre-commit .git/hooks/post-commit

openinvar analyze . --snapshot HEAD
```

Before an agent edits a file, its blast radius is put into context:

```
OpenInvar blast radius for src/models/user_payload.ts: 1 file(s) and 3
reference(s) depend on symbols defined here.
```

And a change that breaks a rule does not reach a commit:

```
$ git commit -m "drop unused email field"
    payload-is-stable no-field-removal [error]
        src/models/user_payload.ts:5
          field 'email' removed from 'UserPayload'

openinvar: commit blocked by a rule in openinvar.toml
         Override once with: git commit --no-verify
```

Only a rule violated on resolved evidence blocks. A missing binary, a missing
graph, a stale snapshot or a timeout all let the commit through and say on
stderr that nothing was checked — a gate that silently stops working is worse
than one that is plainly off. Full details, including the `post-commit` hook
that keeps the baseline fresh, are in
[`agent-configs/hooks/README.md`](agent-configs/hooks/README.md).

## GitHub Action

```yaml
# .github/workflows/openinvar.yml
name: OpenInvar
on: pull_request
permissions:
  contents: read
  pull-requests: write
jobs:
  openinvar:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0   # both sides of the comparison must exist locally
      - uses: JeelGajera/OpenInvar@v0
```

It analyzes the base and head commits, compares them, evaluates
`openinvar.toml`, and posts one comment — updating it on each push rather
than adding another.

```markdown
## OpenInvar

**1 rule(s) violated.** This change is blocked.

### Rules

| Rule | Kind | Verdict |
|---|---|---|
| core-must-not-depend-on-cli | forbid-dependency | **FAIL** |
| payload-is-stable | no-field-removal | pass |

**core-must-not-depend-on-cli** (forbid-dependency, error):

- `crates/openinvar-core/src/ir.rs:12` — core/ir.rs -> cli/main.rs (imports)
```

The verdict is the first line, so a reader who stops there still has the
answer. Rules that could not be decided are named in the comment rather than
only in the exit status, and never fail the job.

| Input | Default | Notes |
|---|---|---|
| `version` | `latest` | A release tag, or `source` to build from the checkout |
| `base-ref` | PR base | What to compare against |
| `rules` | `openinvar.toml` | |
| `comment` | `true` | |
| `fail-on-violation` | `true` | Undecided rules never fail the job |

## MCP Integration

Start server:
```bash
openinvar serve --stdio
```

Six tools, deliberately. A large tool surface degrades an agent's ability to
pick the right one, so each of these answers a question the others cannot:

| Tool | Answers |
|---|---|
| `get_blast_radius` | What breaks if I change this symbol? |
| `get_dependencies` | What does this symbol depend on? |
| `get_symbol_usages` | Where is this used, including under aliases? |
| `graph_diff` | What did this change break? |
| `check_rules` | Does this change violate the rules this repository wrote down? |
| `refresh_graph_index` | Re-analyze after changes |

The three query tools take `kinds` and `min_resolution` filters. Results carry
their resolution: an answer marked structural was matched within one file and
cannot see across files, so an empty result from it is not evidence that
nothing depends on the symbol. `graph_diff` and `check_rules` read snapshots
recorded by `openinvar analyze --snapshot` and never re-analyze, so their answers
depend only on the revisions named.

Agent and MCP setup templates are in [`agent-configs/`](agent-configs/).

The folder includes ready-to-use examples for:
- `AGENTS.md`
- Claude Code `CLAUDE.md`
- Claude Code Skills
- Cursor rules
- GitHub Copilot instructions
- Gemini guidance
- Antigravity-style rules/workflows
- MCP configs for Cursor, Claude Code, Antigravity and Codex

## Language Support

Supported now:

| Language | Tier | Extensions | Resolves |
| --- | --- | --- | --- |
| TypeScript / JavaScript | 1 | `.ts` `.tsx` `.js` `.jsx` `.mts` `.cts` `.mjs` `.cjs` | `tsconfig` paths, barrel re-exports, decorator DI |
| Framework files | 1 | `.vue` `.svelte` `.astro` | script blocks within the component |
| Python | 1 | `.py` `.pyi` | relative imports, `__init__` re-export chains, Pydantic / Django / dataclass fields |
| Rust | 1 | `.rs` | Cargo workspaces and per-crate module trees, `use` groups and aliases, trait impls, `#[derive]` |
| Go | 1 | `.go` | package imports via `go.mod`, structural interface satisfaction |
| C | 1 | `.c` `.h` | `#include` resolution, `typedef` aliases |
| C++ | 1 | `.cpp` `.cc` `.cxx` `.hpp` `.hxx` `.hh` | `using` aliases, base classes, namespace-qualified names |
| Java | 1 | `.java` | single-type, on-demand and static imports, same-package visibility, declared types, inherited members |
| C# | 1 | `.cs` | `using` namespaces and aliases, `using static`, enclosing-namespace visibility, partial types across files, declared types, inherited members |

Tier 2, carried by the released binary and reported as tier 2 wherever they appear:

| Language | Tier | Extensions |
| --- | --- | --- |
| Ruby | 2 | `.rb` |
| PHP | 2 | `.php` |

Every Tier 1 adapter resolves import aliases and attributes member access to
the type a value was declared as, so `payload.user_id` is recorded against
`UserPayload` however the variable was named.

**What Tier 2 gives you, plainly.** Symbols, and references *within a single
file*. You can locate a definition, list what a file declares, and see
intra-file usage.

**What it does not.** No import resolution, no aliases, no declared types — so
no cross-file reference of any kind. A tags query reports that a call to `foo`
happened; it does not say which `foo`, and guessing by name across a repository
is the bug OpenInvar exists to avoid.

**Tier 2 is a status, not a permanent class.** Java and C# both shipped as
Tier 2 and are now Tier 1: same grammars, same binary, but with import
resolution, declared-type binding and a supertype walk behind them. The tier
says what has been *built* for a language, not how much it is valued. Ruby is
the honest caveat — autoloading, monkey-patching and `method_missing` make a
large share of its references undecidable statically, so it may stay Tier 2
rather than be promoted on a claim the analysis cannot support.

**Gates do not fire on Tier 2 regions.** Not "are discouraged from" — they do
not. `check` reports an undecided verdict rather than a pass, `tests` refuses to
call its selection complete, and every audit detector is restricted to files a
Tier 1 adapter resolved, with the framework dropping an out-of-scope finding
even when a detector forgets to check. A structural region cannot support a
conclusion drawn from the *absence* of a reference, because the reference would
never have reached the graph.

### Known limits

Being explicit about these is more useful than a feature list:

- **An empty blast radius is only "safe to modify" when the whole graph is
  resolved.** Tier 2 (structural) analysis sees one file at a time, so a
  reference from a structural region never reaches the graph. When any edge in
  your graph is structural, `blast-radius` reports the empty result but
  withholds the safety verdict and names the files it could not resolve across.

- **Call and instantiation edges cover every Tier 1 language, at different
  depths.** A direct call records `Calls` when the name binds to a symbol the
  file can see, and `Instantiates` when the thing called turns out to be a type;
  a callee that binds to nothing records no edge rather than a guess, and
  neither does a call whose only available target is a third-party package. In
  Rust, `Foo::new(..)` names the method that runs rather than the type, and
  `Foo {..}` is construction outright; in Go, `pkg.Func(..)` is recorded because
  that is how every cross-package call is written, and `Foo{..}` is
  construction. `obj.method()` records no call edge — it is a property access on the receiver's declared type instead.
  C and C++ are the narrowest: only a bare `foo(..)` and `new Foo(..)`, because
  C++ methods are not symbols in this graph — but a bare call does cross
  translation units through a header prototype. Tier 2 languages see calls within a
  single file only. A query filtered to a kind no edge in your graph carries is
  reported as such, so an empty result is never mistaken for "nothing calls
  this".

- **A C call through a header prototype reaches the definition.** C splits a
  call across two files: the caller includes a header that declares the
  function, and the definition lives in a `.c` file the caller never sees. The
  caller attaches to the definition, so `blast-radius` on the definition finds
  it. The link is made when a header declares a name and exactly one file both
  defines that name and includes that header — an agreement between two files,
  not a name matched across the repository. Two definitions, or a definition
  whose file does not include the header, leave the call unlinked rather than
  guessed at.

- **A method call is only attributed where the receiver's type is declared.** A
  parameter or field gives the receiver a type, so `u.Greeting()` is recorded as
  a property access on that type. A local bound by inference — Go's `u := f()`,
  and the same shape elsewhere — is not tracked, so the method call reaches the
  graph as nothing at all.

- **Test detection is by file convention only.** `_test.go`, `*.test.ts`,
  `test_*.py`, a Rust crate's `tests/` — the rule each language's own tooling
  uses. A Rust `#[cfg(test)] mod tests` inside an ordinary source file is *not*
  recognised, and the symbols inside such a module are not indexed at all, so
  those tests produce no `tests` edges and `openinvar tests` will not suggest
  them. Integration tests under `tests/` are covered.

- **`openinvar tests` licenses an omission, and says when it cannot.** Naming the
  tests that cover a change is a claim that the ones left out cannot fail. Exit
  `3` means something could be missing — an uncovered changed symbol, a
  structural region, or a test selected on weak evidence — and the full suite
  should run. A test the change itself modified is reported apart from coverage
  and does not mark anything covered.

- **Three audit detectors ship; three are held back, and say why.**
  `assertion-removal` has no signal at all: assertion calls are not in the
  graph, since Rust's assert macros are unexpanded token trees and framework
  calls resolve to nothing. `scope-creep` has no denominator — OpenInvar is never
  given an intended scope. `special-casing` needs control-flow analysis, and the
  graph models references between symbols rather than flow inside one. `openinvar
  audit` names every check that did not run, because an absent check otherwise
  reads as a passing one.

- **`test-tampering` fires on coverage disappearing, not on a test changing.**
  Changing a function and updating its test together is ordinary work; a
  detector firing there would be switched off in a week. It reports a test that
  *stopped* covering a symbol which changed in the same diff.

- **`openinvar context` is worth less against a text search than against reading
  files.** Orienting on `RepoIR` here costs ~420 estimated tokens against
  ~57,900 for reading those thirty files whole — but only ~670 for `rg -l`,
  which answers the same "which files touch this" question. The signature
  skeleton, the part a text search cannot produce, lands for 1 of those 31
  entries: inbound edges to a widely-used type are dominated by imports, which
  adapters attribute to the file's module symbol rather than the function using
  it. Useful for a narrow neighbourhood; thin for a widely-imported type. Token
  figures are byte-based estimates, not a tokenizer's count.

- **`no-cycles` reports a language's own module structure as a cycle in Rust,
  and in Go at file level.** It is built for the cycles that break something:
  ESM bindings that resolve `undefined`, Python `ImportError`, tangled C++
  headers. Rust is different in two ways that both show up here. A crate root
  declares `pub mod x;` while `x.rs` refers back with `use crate::…`, which is
  a cycle by the graph's reckoning and ordinary code by Rust's. And where
  `crate::ir::Thing` cannot be resolved through the module's exports, the edge
  attaches to the crate root instead — the documented fallback below — which
  closes the loop again.

  Run on this repository, the rule reports five file-level cycles, every one of
  them a `lib.rs` ↔ submodule pair. `level = "module"` collapses most of that
  but still reports parent ↔ child module directories. So: use it on
  TypeScript, JavaScript, Python and C/C++, where a cycle means something is
  broken. On Rust and Go, scope it narrowly or treat it as advisory. This
  repository does not gate on it, for that reason.

- **Imports resolve within one language.** A Python module importing a
  TypeScript file through a build step is not linked.
- **Chained access is attributed to the first receiver only.** In `a.b.c`, the
  `c` is not attributed to the type of `a.b`, which would require field-type
  resolution.
- **C++ templates are parsed but not instantiated.** `vector<Foo>` records a
  reference to `Foo`; it does not model what the instantiation generates.
- **Go structural matching is per-package.** A type satisfying an interface
  declared in another package is not reported, because matching every method
  set against every interface repository-wide produces far more noise than
  signal.
- **Rust macro bodies are token trees.** Field access inside `format!` and
  friends is recovered by scanning tokens; more elaborate macro-generated code
  is not expanded.
- **Fully-qualified paths used inline are not resolved.** A type written out in
  place — `openinvar_core::ir::RepoIR` in a signature, with no `use` bringing it
  into scope — records no edge. Resolution binds names through a file's import
  table, and a path like that never enters it. Bring the type into scope with a
  `use` and it resolves normally.


### Tiers

**Tier 1 — resolved.** Imports, aliases and declared types are resolved, so
member access is attributed to the type a value was declared as. Safe to gate
CI on.

**Tier 2 — structural.** Symbols and intra-file references, extracted from the
grammar's own `tags.scm`. No cross-file import resolution, no aliases, no
declared types. Genuinely useful for locating symbols and for intra-file blast
radius — and explicitly not something to gate on. A tags query reports that a
call to `foo` happened; it does not say which `foo`, and guessing by name
across a repository is the bug OpenInvar exists to avoid.

`openinvar status` reports the tier of every language your build carries, and the
share of edges that resolved — overall and per language. An enforcement tool
that tells you "91% of references in this repository resolved, and here is what
it could not" is worth more than one implying completeness.

Queries take `--min-confidence resolved` to restrict an answer to edges bound
through imports, aliases and declared types. The threshold is applied while
traversing, not to the results, so nothing is reached by way of an edge below
it. On a Tier 2 repository that correctly returns nothing.

Tier 2 today: Ruby, which has its own feature for a narrowed source build and
is in the released binary.

Still planned as Tier 2: Kotlin, Swift, Scala, SQL, Lua, Bash. These are
not held up by OpenInvar's architecture but by the grammar crates. Adding one
needs a crate that works against the `tree-sitter` version OpenInvar pins and
*exposes* its `tags.scm` as a `TAGS_QUERY` constant. Shipping the file is not
enough: a language's spec hands over the crate's constant — Ruby's is
`Some(tree_sitter_ruby::TAGS_QUERY)` — and nothing reads the packaged file.

The version conflict that blocked every one of them, PHP included, is gone.
Every crate below now
depends on `tree-sitter-language` rather than on the runtime, so a grammar and
a runtime no longer have to agree on a version. Surveyed against the index
after the move to `tree-sitter` 0.27 and verified by parsing a sample with
each:

| Language | Crate | Parses | `TAGS_QUERY` |
| --- | --- | --- | --- |
| Swift | `tree-sitter-swift` 0.7 | yes | yes, 7 patterns |
| Lua | `tree-sitter-lua` 0.5 | yes | yes, 5 patterns |
| Scala | `tree-sitter-scala` 0.26 | yes | ships `queries/tags.scm`, exports no constant |
| Kotlin | `tree-sitter-kotlin-ng` 1.1 | yes | no tags query |
| SQL | `tree-sitter-sequel` 0.3 | yes | no tags query |
| Bash | `tree-sitter-bash` 0.25 | yes | no tags query |

PHP is added, and is in the Tier 2 table above. Swift and Lua are ready to add
on the same evidence. Scala needs one constant upstream, or a
vendored copy of a query its own crate already contains — which trades "adding
a language is a dependency" for a file to keep in step with a grammar that
moves. Kotlin, SQL and Bash need a tags query to exist at all before there is
anything to run.

Kotlin is the one place the crate matters: the original `tree-sitter-kotlin`
still pins `tree-sitter >=0.21, <0.23` and cannot be used at all, while
`tree-sitter-kotlin-ng` builds against the current runtime — and neither ships
a tags query, so the choice is moot until one does.

## It also answers graph questions

The graph that `audit` and `check` run on is queryable directly. This is not
what OpenInvar is for — dedicated code-graph tools do it well and there is no
reason to switch to this one for it — but the answers are here and they are
alias-aware.

```bash
openinvar query blast-radius UserPayload   # what breaks if I change this
openinvar query usages UserPayload         # every caller, aliases included
openinvar query deps UserPayload           # what it depends on
openinvar impact src/models/user.ts        # what depends on one file
openinvar context UserPayload              # a minimal working set for orienting
```

Each takes `[--file <path>] [--depth <n>] [--kind <kind>]`, and
`--min-confidence resolved` to exclude structural edges.

### Filtering

OpenInvar honors `.gitignore` by default. If a symbol is missing, check whether it
lives in an ignored folder such as `dist/`, generated output, or scratch files.

Override filters when needed:

```bash
openinvar analyze . --no-gitignore
openinvar analyze . --include "src/**/*.ts"
openinvar analyze . --exclude "tests/**"
openinvar watch . --include "packages/api/**/*.ts"
```

For MCP clients, `refresh_graph` accepts:

- `path`
- `respect_gitignore`
- `include`
- `exclude`

Example:

```json
{
  "path": ".",
  "respect_gitignore": false,
  "include": "src/**/*.ts",
  "exclude": "tests/**"
}
```

### Filtering by relationship kind

Every query can be narrowed to particular kinds of reference:

```bash
# only what imports it, not what merely inherits from an importer
openinvar query blast-radius UserPayload --kind imports

# repeatable
openinvar query usages UserPayload --kind imports --kind re-exports
```

Kinds: `imports`, `calls`, `extends`, `implements`, `uses-type`,
`accesses-property`, `re-exports`, `instantiates`, `tests`.

Filtering applies to the traversal, not to the result, so an excluded kind
also stops the walk continuing through it. A filtered query reports the filter
it used, and an empty filtered result is never described as safe — only part
of the graph was searched.

`tests` is derived rather than parsed: a test file's references into non-test
code are restated under it, so `--kind tests` answers "what covers this" as a
filter over the graph rather than a separate traversal. The underlying `calls`
or `imports` edge is kept as well, so a query for callers still finds tests.

Which kinds a given repository contains depends on its languages. A filter
matching no edge in the analyzed graph is reported as such rather than
returning a silent empty result.

### Context

```bash
openinvar context RepoIR
openinvar context --diff --base HEAD --head worktree --budget 2000
```

Emits the symbol, what it depends on, what depends on it, and a signature for
each — the shape of the neighbourhood rather than its contents — and reports
what that cost against reading the same files whole.

**Measured on this repository**, and worth reading with its caveat:

| Question | Estimated tokens |
|---|---|
| `openinvar context RepoIR` | ~420 |
| Reading those 30 files whole | ~57,900 (**138x** more) |
| `rg -n RepoIR` | ~4,050 (10x more) |
| `rg -l RepoIR` — the file list alone | ~670 (**1.6x** more) |

The 138x figure is real but flatters the feature. An agent orienting itself
does not read thirty files whole; it runs a text search. Against `rg -l`, which
answers the same "which files touch this" question, the saving is 1.6x — and
the part a text search cannot produce, the signature skeleton, is delivered for
only 1 of those 31 entries, because inbound edges to a widely-used type are
attributed to each file's module symbol rather than to the function that uses
it.

So: useful for a narrow neighbourhood where signatures land, thin for a
widely-imported type. This is the capability that erodes as native code search
improves, and it is deliberately last in the release for that reason.

Token figures are byte-based estimates, not a tokenizer's count. OpenInvar
vendors no tokenizer: one is model-specific, and a figure that moved with
somebody's model would not be reproducible.

A `--budget` drops the outermost hops first and reports how many symbols it
omitted, rather than truncating silently.

### Machine-readable output

`openinvar analyze --json` writes the full analysis to stdout as a single JSON
document and suppresses all progress output, so it can be piped directly:

```bash
openinvar analyze . --json > analysis.json
```

The document carries a `schema_version`. Pin it: fields may be added within a
version, and anything a consumer could observe breaking bumps it.

Output is deterministic — the same input produces byte-identical bytes, which
is what makes two analyses safe to diff.

## Building for one language

**There is one released binary per platform, and it carries every language.**
Nothing to choose, and the coverage figure above is the one your binary
reports.

A source install can still be narrowed, which is worth doing only if binary
size actually matters to you:

```bash
cargo install openinvar-cli --no-default-features --features python
cargo install openinvar-cli --no-default-features --features tier1
```

Features: `typescript` (includes JavaScript), `python`, `rust`, `go`, `c`
(includes C++), `java` and `csharp` are Tier 1, and `tier1` is the seven
together. `ruby` is Tier 2. `full` is everything and is the default.

`openinvar status` and `--help` report what your build can analyse, and a build
skips files in languages it does not carry rather than failing on them — which
also means a narrowed build reports coverage over fewer files. A `tier1` build
reports 100% on this repository where the released binary reports 99.8%, not
because it resolves more but because it never looks at the Tier 2 files.

## Build & Test

```bash
cargo build --release
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

Nothing to set in the environment and no system libraries to install. The store
is SQLite, compiled from the bundled amalgamation — one C file. A CI job builds
with the newest GCC the runner offers, and with no `CC`/`CXX` at all, so that
stays true.

## Changelog

Release history is in [CHANGELOG.md](CHANGELOG.md).

## Contributing

Contributions are welcome — [CONTRIBUTING.md](CONTRIBUTING.md) covers the build,
the checks CI runs, and what a mergeable change looks like. Two things shape
everything else in this repository, so they are worth knowing before you start:

- **Determinism is non-negotiable.** No LLM participates in graph construction
  or in any gating decision, and anything that reaches output needs a stable
  ordering key.
- **Limits are documented, not smoothed over.** A blind spot written down is a
  feature; one a user discovers is a bug.

The two most common contributions each have a short guide:
[adding a language](CONTRIBUTING.md#adding-a-language) — a module and a Cargo
feature, not a new crate — and
[adding a rule kind](CONTRIBUTING.md#adding-a-rule-kind), where most of the work
is being honest about what the graph cannot see.

Participation is covered by the [Code of Conduct](CODE_OF_CONDUCT.md).

## Security

Please report vulnerabilities privately rather than in a public issue.
[SECURITY.md](SECURITY.md) has the process, and sets out what is in scope —
OpenInvar's security surface is reading code it did not write, frequently in CI,
frequently from outside the project. It never executes what it analyses.

## License

Apache-2.0 — see [LICENSE](LICENSE)
