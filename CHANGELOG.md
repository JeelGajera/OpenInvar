# Changelog

All notable changes to OpenInvar are documented here.

Entries written before the rename still say `graphyn`, `graphyn-core`,
`.graphyn/` and so on. They are deliberately left as written: those were the
real names at the time, and rewriting them would make this file quietly
dishonest about what shipped, and under what name.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Reference-level coverage — the share of references that actually bound.**
  `status` reported resolution coverage as though it measured how much of a
  repository OpenInvar understood. It does not. It is the share of *recorded
  edges* that resolved, and an edge exists only once something bound it — so a
  reference the analysis could not place left nothing to count, never entered
  the denominator, and failing to bind more references made the number go
  **up**. The project's own evaluation found this and registered a replacement
  before measuring anything with it; this implements it.

  Every Tier 1 adapter now counts the references it could not place, at the
  point where it already decides to drop them. `status` reports both figures:

  ```
  Resolved           99.8% (5980 of 5992 edge(s))
  References bound   69.6% (5980 of 8597 reference(s))
    2617 reference(s) named in source bound to nothing in the graph.
  ```

  Measured on this repository. The gap between 99.8% and 69.6% is what was
  invisible.

  **Nothing about the analysis changed** — the graph is byte-identical and the
  golden diff is two new fields per file and no altered edge. That was the
  hard part: an earlier attempt had the adapters stop discarding unresolved
  references so one central counter could see them, which changed seven
  adapters' output and broke eleven tests asserting real properties, among them
  "a builtin call records no edge". The count is now taken where each adapter
  already makes its decision, and a test asserts the analysis is unaffected.

  An unbound reference is not automatically a defect: a type from a package
  whose source was never analysed is indistinguishable, to an adapter, from one
  it should have found and missed. Where an adapter can positively identify a
  reference as living outside the repository — a language builtin, a
  third-party package — it is not counted. Where it cannot tell, it is counted,
  because a measure that excluded what it could not classify would be reporting
  its own confidence rather than its coverage.

  Snapshot format version 5 carries the counts; a version 4 snapshot reads as
  "this graph does not know" rather than as zero.

- **C# is now Tier 1.** The last of the two languages that shipped structural,
  and the harder one. Its module said plainly that nothing in it resolved an
  import, followed an alias, or bound a declared type; all three are here now,
  and gates may act on C#.

  Three things C# does that Java does not, each of which drives a rule:

  - **`using` names a namespace, not a type.** There is no single-type import,
    so every `using` is the on-demand form and the ambiguity Java reaches only
    through wildcards is C#'s ordinary case. Two `using` directives offering one
    name is what the compiler rejects, so the reference is **dropped rather than
    guessed**.
  - **Enclosing namespaces are in scope.** Inside `namespace A.B.C`, a type in
    `A.B` or `A` resolves with no `using` at all, innermost first. A resolver
    searching only the exact namespace would drop most references in a
    deeply-namespaced project.
  - **A type can be declared across files.** `partial class Report` in two files
    is one type with the union of its members, so a call into the other half has
    to resolve. Members are indexed by the owner's fully-qualified name rather
    than by declaration, which makes that fall out instead of needing a special
    case.

  Alias directives, `using static`, inherited members via a supertype walk, and
  declared-type binding for property attribution are all resolved. Each rule has
  a test that fails when the rule is inverted, and the partial-class test checks
  **both** directions deliberately: an index keyed by declaration still answers
  one of them, so a one-way test would pass without the halves ever merging.

  `var` is read, not inferred. `var x = new Foo()` states its type in the
  initialiser, so binding it is reading; `var x = Something()` does not, and
  working it out would mean inferring a return type — a second type system
  beside the real one, disagreeing with it exactly where it matters. The second
  form binds to nothing and records no edge.

  A `using` of a namespace the repository itself declares is dropped rather than
  recorded as an external package: the concrete edges to the types it brought in
  carry the real information, and marking it external would put the repository's
  own code in its own third-party list. A `using` of a namespace declared
  nowhere in the tree still becomes an external package node.

  Measured on this repository, C#'s resolution coverage moves from
  `0.0% of 2 edge(s)` to `100.0% of 12 edge(s)`, and `fixtures/adapter-csharp`
  joins the golden corpus with 8 cross-file edges that Tier 2 could not record
  at all. No other fixture's golden moved.

  Ruby is now the only Tier 2 language, and remains the honest caveat:
  autoloading, monkey-patching and `method_missing` leave a large share of its
  references undecidable statically, so it may stay Tier 2 rather than be
  promoted on a claim the analysis cannot support.

- **An evaluation against real repositories**, in `eval/`. The claims this
  project makes were measured only against fixtures it wrote itself, which
  resolve at 100% because they were built to — that proves the code does what
  its author expected, not that it survives code nobody here wrote.

  Four ordinary, widely-used repositories (rust-lang/log, spf13/cobra,
  pallets/click, google/gson), eight consecutive commits each: 32 revisions and
  28 commit pairs. Thresholds were fixed in `eval/criteria.md` before the run,
  and the runner and the summariser are separate files so a threshold cannot be
  moved in the same edit that produces the result it judges.

  **Determinism held 32 of 32, byte for byte**, each revision analysed twice
  into worktrees on different paths. **No error-severity audit finding on any
  of the 28 commit pairs**, against a ≤10% threshold. Slowest revision 0.71s
  for 313 files and 18,989 edges.

  The run's substantive finding is that the resolution-coverage criterion does
  not measure what it was written to measure: `dispatch` stamps every Tier 1
  edge `Resolved`, so a repository with no Tier 2 files reports 100% by
  construction. It is a tier classification, not a resolution rate. Recorded as
  such rather than scored as the pass it numerically was, with the criterion
  left unedited and a replacement proposed for the next run.

  An independent check, against the source text rather than anything OpenInvar
  reports about itself, found gson's 2432 `import` statements produced exactly
  2432 `Imports` edges — import resolution verified complete on that corpus,
  with reference-level drop rate still unmeasured.

- **Java is now Tier 1.** It shipped as the demonstration Tier 2 language, with
  its own module saying what promotion would take:

  > Promoting Java to Tier 1 means implementing import resolution, alias
  > resolution and declared-type binding for it.

  That is what this adds: a parser, an extractor, a scope analyzer binding each
  receiver to its declared type, and a resolver following Java's own name
  resolution order. Gates may now act on Java, and `status` reports it as
  `tier 1 (resolved)`.

  The order is the substance. A type declared in the file shadows an import;
  the file's own package is searched before any on-demand import; two wildcard
  imports offering one name is an ambiguity `javac` rejects, so the reference is
  **dropped rather than guessed**; a method declared on the enclosing type
  shadows a statically imported one; and an inherited method resolves by walking
  supertypes across files rather than by matching the name wherever it appears.
  Each of those has a test that fails when the rule is inverted.

  Nothing outside the repository records an edge. `String` and `List` are not
  declared anywhere in the analysed tree, so any edge to them would be invented
  — and `dispatch` stamps every Tier 1 edge gate-safe on the way out, so an
  invented one would be marked as evidence a gate may act on. Imports that leave
  the repository become external package nodes instead, which keeps the
  dependency visible without fabricating a symbol.

  Measured on this repository, Java's resolution coverage moves from
  `0.0% of 2 edge(s)` to `100.0% of 14 edge(s)`, and `fixtures/adapter-java`
  joins the golden corpus with 10 cross-file edges that Tier 2 could not record
  at all.

  **Tier 2 is a status, not a permanent class**, and this is the evidence. Ruby
  remains the honest caveat: autoloading, monkey-patching and `method_missing`
  leave a large share of its references undecidable statically, so it may stay
  Tier 2 rather than be promoted on a claim the analysis cannot support.

  This repository's own `no-new-god-symbol` threshold moves from 180 to 200,
  which its comment calls the justified case rather than the silencing one:
  every Tier 1 adapter matches on the relationship vocabulary, so
  `RelationshipKind` gains about fifteen references per language — 169 to 184
  for Java. The figure tracks the language count by design. Caught by the
  repository gating on its own binary, not by review.

- **`assertion-removal`** — a fourth audit detector, for a test that kept its
  coverage but stopped checking anything.

  `test-tampering` catches coverage disappearing. This catches the subtler
  move: the test still runs, still references the symbol, and the `assert_eq!`
  has become a call with its result dropped. The suite passes and the coverage
  graph is unchanged, so from the graph alone the two revisions were identical.

  The detector was specified early and **held back**, with the reason recorded
  in `held_back()`: assertion calls are not in the graph, and counting them
  would mean parsing source text outside it, which is a second source of truth.
  That was right about the constraint and pointed at the fix — the problem was
  counting them *somewhere else*. Assertions are now counted during the same
  tree-sitter parse the symbols come from and travel with them, so there is
  still exactly one source of truth.

  Only unambiguous forms count: Rust's `assert*` macros, Python's `assert` and
  the `unittest` `assertX` family, `expect`/`assert` in TypeScript and
  JavaScript, testify plus `t.Error`/`t.Fatal` in Go, and `assert` in C and
  C++. Not `unwrap()`, which is ordinary code as often as it is a check — a
  count that moved when one was refactored away would accuse someone over
  ordinary work.

  Three conditions must all hold before anything is reported: the test
  survived, it still covers the symbol, and the covered symbol changed in this
  diff. Losing coverage is `test-tampering`'s finding, and reporting both for
  one event would say the same thing twice.

  **A count is only meaningful where both revisions counted the language.**
  Zero assertions in a language this build cannot read means unknown, not none.
  Auditing against a snapshot recorded before counts existed reports none
  counted, so without that gate an upgrade would report every test in a
  repository as emptied at once — the loudest possible false accusation, on the
  first run after an upgrade.

- **Snapshot format version 4**, carrying assertion counts and, per file,
  whether its language was counted. Versions 1 to 3 read back as having counted
  nothing, which is what makes the upgrade decline rather than accuse.

### Changed

- **The `tier1` build feature now means what Tier 1 means.** It listed five
  languages and its comment called them "the languages whose imports, aliases
  and declared types are resolved" — which stopped being true when Java was
  promoted, and `full` still named `java` and `csharp` separately as though they
  were not. `tier1` is now the seven resolved languages and `full` is `tier1`
  plus `ruby`. A `--no-default-features --features tier1` build therefore
  carries Java and C# and reports 100% resolution on this repository, against
  99.8% for the released binary — the difference being the Ruby files it never
  looks at, not anything it resolves better.

- **Dependencies updated**: `thiserror` 1 → 2, `dashmap` 5 → 6, `toml` 0.8 →
  1.1, `actions/checkout` v4 → v7, `softprops/action-gh-release` v1 → v3, and
  the lockfile refreshed within existing constraints. No source change was
  needed for any of them, and the golden snapshots are unchanged — the
  dependency updates move no analysis output.

  `tree-sitter-python` 0.21 → 0.23 is **not** included. Its 0.23 binding
  replaces `language()` with a `LANGUAGE: LanguageFn` constant, and `LanguageFn`
  needs the tree-sitter runtime at 0.23 while this workspace is on 0.22 with
  every other grammar on 0.21. Taking it alone does not compile. Moving the
  whole ecosystem (tree-sitter is at 0.27, the grammars at 0.23–0.25) is an API
  migration across ten grammars that also moves parse trees, so it belongs in
  its own change where the golden diff can be read as evidence rather than
  noise.

- **`analyze --at <rev>`** — analyse the tree at a revision instead of the
  working tree.

  ```bash
  openinvar analyze . --at 4f2a1c9
  openinvar diff . --base 4f2a1c9 --head worktree
  ```

  `--snapshot` records whatever is on disk; `--at` fetches a revision and
  records that, so history can be built without a checkout per commit.

  The revision is checked out into a temporary worktree, not in place:
  uncommitted work is untouched and `HEAD` does not move. `git worktree` rather
  than `git archive` because archive applies `export-ignore` from
  `.gitattributes`, and a repository using it would be analysed with files
  silently absent — a graph missing symbols that are really there, which is the
  confidently wrong answer a gate must never give. Cleanup is in a `Drop` guard
  so an error between checkout and completion cannot leak a directory or an
  entry in `.git/worktrees`.

  **`--at` does not replace the working graph.** `query`, `check` and `audit`
  read it when nobody names a revision, so overwriting it would leave every
  later command answering about a past revision without saying so.

  Determinism holds across the temporary directory: the checkout's path never
  reaches the result. The graph records relative paths only, and the report's
  single absolute path is restated as the repository, so two runs over one
  revision are byte-identical.

  `worktree` is refused rather than accepted as a no-op — it names the working
  tree, and recording that under a name every reader takes for a commit is not
  something a later `diff` could detect. `--at` and `--snapshot` conflict, since
  `--at` already determines the name.

  `revision::resolve_commit` is added beside `resolve` for revisions that must
  name a commit, so the error does not offer `worktree` to a caller that will
  then reject it.

- **Contributor and community documentation.** `CONTRIBUTING.md`,
  `SECURITY.md`, `CODE_OF_CONDUCT.md`, issue templates, a pull request
  template, and a Dependabot configuration.

  `CONTRIBUTING.md` writes down what previously lived only in reviewers' heads
  or in workflow comments: every check CI runs and how to run it first, the
  golden snapshot workflow and why an empty diff is evidence, the dogfooding
  invariant, the per-language slim build, and the four-verdict model a new rule
  kind has to reason within. A comment in `ci.yml` had already referred to a
  CONTRIBUTING that did not exist — the GCC pin that broke contributors'
  builds lived only in the workflows, which is how it went unnoticed.

  `SECURITY.md` routes reports to GitHub's private advisory flow and states the
  threat model rather than a generic promise: OpenInvar's surface is reading
  code it did not write, usually in CI, often from outside the project. In
  scope are parser crashes on crafted input, path traversal, store escape, the
  installers, the MCP server, and any input crafted to make a gate report a
  pass it has not earned. Out of scope are vulnerabilities in the code
  OpenInvar analyses — it is not a scanner and does not claim to be. It never
  executes what it reads, and nothing leaves the machine.

  The issue templates include one for proposing a rule kind, built around the
  questions that decide whether a kind can exist honestly — chiefly which
  direction its uncertainty runs, since an unresolved edge is a possible
  violation for `forbid-dependency` and a possible *satisfier* for
  `requires-dependency`.

- **`requires-dependency` and `no-orphans`** — the remaining claims about
  absence, added together so their uncertainty semantics cannot drift apart.

  ```toml
  [[rule]]
  name = "handlers-log"
  kind = "requires-dependency"
  from = "src/http/handlers/**"
  to   = "src/audit/**"

  [[rule]]
  name  = "no-dead-modules"
  kind  = "no-orphans"
  scope = "src/**"
  roots = ["src/main.ts", "src/index.ts"]
  ```

  `requires-dependency` inverts `forbid-dependency`, and the inversion is the
  whole subtlety. There, an unresolved edge might be a violation; here it might
  be the edge that *satisfies* the rule. A scoped symbol with no resolved edge
  to the target but some unresolved outbound edge is undecided, not in breach.

  `no-orphans` is the weakest claim in the vocabulary and defaults to `warn`.
  Orphanhood is pure absence, so any edge that did not resolve could be the
  reference that makes a symbol reachable — on a repository with structural
  regions it is undecided nearly always, and promoting it to a gate would claim
  a certainty the graph cannot supply. `roots` exists because an entry point is
  unreferenced by definition, and a rule that reports every `main` is one
  nobody keeps switched on.

  This completes the rule vocabulary: `layers`, `independence`, `no-cycles`,
  `max-fan-in`, `max-fan-out`, `naming-convention`, `requires-test`,
  `requires-dependency`, `no-orphans`, `forbid-dependency`, `forbid-reference`
  and `no-field-removal`.

- **`requires-test`** — the rule no comparable tool can express. `tests` edges
  are derived across every Tier 1 language here; semgrep, dependency-cruiser,
  ArchUnit and import-linter have no equivalent in any language.

  ```toml
  [[rule]]
  name     = "new-api-is-covered"
  kind     = "requires-test"
  symbols  = "src/api/**"
  only     = "function"   # optional
  new_only = true         # judge only what this change added
  ```

  **Direct edges only.** That a test reaches `login`, and `login` references
  `format_token`, is no proof the test ever executes `format_token` — it may
  sit behind an early return, and reporting the symbol covered would be
  claiming certainty from an assumption. It is also the difference between a
  rule and a loophole: an agent that adds a function and needs this green could
  satisfy a transitive version by wiring it into any legacy endpoint that
  already has an integration test, with no unit test and no assertion.

  **An apparent gap is uncertainty, not an accusation.** Test detection is by
  file convention and Tier 2 files produce no `tests` edges at all, so where the
  graph carries structural regions, "nothing covers this" and "nothing this
  build can see covers this" are the same observation. The rule answers
  definitely only on a fully resolved scope.

  `new_only = true` is what makes it adoptable: *every symbol you added is
  covered* is enforceable on a repository that could never pass *every symbol
  is covered*. Without a change to compare against, that form is skipped rather
  than passed.

  Tests are excluded from their own scope — nobody writes a test for a test. A
  test is recognised by originating a `tests` edge rather than by its filename,
  since `is_test_file` lives in the language crate that the engine cannot
  depend on and the graph already carries the answer.

- **`no-cycles`** — the most-requested architecture rule there is, and the one
  a sceptical reader checks for before believing a tool is serious.

  ```toml
  [[rule]]
  name  = "no-import-cycles"
  kind  = "no-cycles"
  scope = "src/**"      # optional; the whole graph by default
  level = "file"        # file (default) or module
  ```

  Cycles report as an ordered walk — `a.ts -> b.ts -> c.ts -> a.ts` — rather
  than as a set, because a cycle whose shape nobody can read is a cycle nobody
  fixes.

  **Two passes, because one would be dishonest.** Running Tarjan over resolved
  edges and calling a clean result "no cycles" is wrong: an edge that did not
  resolve could be the one that *closes* a cycle the resolved edges leave open.
  So proving a scope acyclic means having resolved every edge inside it. Where
  any in-scope node has an unresolved outbound edge, the verdict is
  inconclusive rather than a pass. A cycle already found still reports as a
  violation — weak evidence elsewhere does not make it less of a cycle.

  `level = "symbol"` is **refused**, with a reason rather than silently. Mutual
  recursion between functions is correct, ordinary code — recursive-descent
  parsers, visitors, state machines — so a symbol level would fire on every
  tokenizer in existence and be switched off within a week. `level = "module"`
  exists for Go, where files inside one package reference each other freely and
  the compiler already rejects circular *package* imports.

  **Known blind spot, documented rather than discovered later.** On Rust the
  rule reports the language's own module structure. A crate root declares
  `pub mod x;` while `x.rs` refers back with `use crate::…`, and where that
  path cannot be resolved through the module's exports the edge attaches to the
  crate root — closing a loop that is ordinary Rust. Run against this
  repository it reports five file-level cycles, every one a `lib.rs` ↔
  submodule pair; `level = "module"` collapses most but still reports parent ↔
  child module directories. The rule is for TypeScript, JavaScript, Python and
  C/C++, where a cycle means something is broken. This repository does not gate
  on it, for that reason.

- **`max-fan-out` and `naming-convention`.**

  `max-fan-out` is the mirror of `max-fan-in` and catches the opposite shape:
  fan-in finds the symbol everything consumes, fan-out finds the one that
  consumes everything. It counts **distinct targets** where fan-in counts
  edges — a file importing the same module on three lines reaches one thing,
  not three, and inflating that would make the threshold meaningless. The
  asymmetry is deliberate and documented rather than left to be discovered.

  `naming-convention` holds symbols in a scope to a pattern, optionally
  narrowed to one symbol kind:

  ```toml
  [[rule]]
  name = "handlers-are-suffixed"
  kind = "naming-convention"
  symbols = "src/http/**"
  matches = "*Handler"
  only    = "class"
  ```

  It is the only kind in the vocabulary that can never be undecided. Every
  other rule reasons about edges, and an edge that did not resolve could always
  have been the one that mattered; a name comes from the parse, so even a file
  whose imports resolve to nothing reports the names it declares.

### Changed

- **`severity` now defaults by kind rather than to `error` everywhere.**

  A stated boundary still defaults to `error` — `layers`, `independence`, the
  `forbid-*` pair, `no-field-removal`, `naming-convention` — because a rule
  written without a severity is one someone means to enforce.

  The fan limits now default to `warn`. High fan-in and fan-out are frequently
  intentional: a utility module, an IR type, an entry point. Blocking CI on one
  would stop a developer who added a module before wiring up its callers, and
  the rule would be switched off rather than tuned. A team wanting zero
  tolerance writes `severity = "error"` explicitly, which still wins.

  This changes the behaviour of an existing `max-fan-in` rule written without a
  severity: it now warns where it previously blocked.

- **Two rule kinds for stating an architecture rather than enumerating it.**

  `layers` takes an ordered stack, outermost first. A layer may depend downward
  and on itself; reaching up is a violation. One rule replaces the pairs it
  would otherwise take — a five-layer stack is ten `forbid-dependency` rules
  written by hand, and this repository's own configuration now states its whole
  crate direction in one:

  ```toml
  [[rule]]
  name = "crate-layering"
  kind = "layers"
  layers = [
    "crates/openinvar-cli/**",
    "crates/openinvar-mcp/**",
    "crates/openinvar-store/**",
    "crates/openinvar-lang/**",
    "crates/openinvar-core/**",
  ]
  ```

  `independence` takes a set of siblings that may not reference each other in
  either direction — the monorepo rule, where feature packages should only meet
  through shared code.

  Both are evaluated natively rather than desugared into pairwise
  `forbid-dependency` calls. One declared rule stays one reported row, a
  violation can name the layers and their ordinals rather than two globs, and
  uncertainty is handled correctly:

  **The outermost layer is never uncertain.** It may depend on everything, so
  an edge that could not be resolved out of it cannot be an upward violation —
  there is nothing above it to violate. A pairwise desugaring only sees "a
  scope had a weak edge" and would report the rule undecided on evidence that
  could never have mattered. Every other layer's unresolved edges do count.
  `independence` has no permitted direction, so every module's do, counted once
  per edge rather than once per module pair.

  A stack of fewer than two entries, a repeated entry, and a glob that does not
  compile are all rejected when the file is read.

### Fixed

- **A rename now has to be evidenced, so unrelated symbols stop being paired
  as one.** `openinvar diff` reported renames between functions that had
  nothing to do with each other. On the C# Tier 1 change its own PR comment
  claimed five, among them:

  ```
  every_edge_is_structural -> a_method_of_the_enclosing_type_wins_over_a_static_using
  the_spec_reports_tier_two -> a_type_outside_the_repository_records_no_edge
  ```

  Nothing was renamed. Those tests were deleted and different ones written. A
  reviewer reading that report is told deleted tests survived under new names,
  which is the tool asserting something untrue about the change.

  The cause is that `renames_to` decided a rename by substituting the old name
  for the new and comparing signatures, and a signature is the declaration line
  alone. A Rust test records as `fn the_name() {`, so with the name substituted
  away every no-argument function matches every other one — and a test file is
  hundreds of them. The predicate was sound; what it assumed was evidence was
  not.

  A pairing whose *name* changed now also requires the signature to say
  something besides the name: the residue left when the name is removed must
  still contain a parameter, a type, a base class, or a literal. One bare token
  is the declaring keyword and nothing else.

  On the commit that exposed it, "renamed or moved" goes from 6 to 1 — the one
  remaining being a genuine move, the same name in a new file — and the five
  test functions are reported as the removals they are.

  The cost, stated rather than discovered: renaming a no-argument function is
  no longer recognised as a rename and reads as a removal plus an addition.
  That is the direction this module already chose, in its own words — inventing
  a rename is worse than missing one. Moves are unaffected, since there the
  name is itself the evidence.

- **No unresolved reference reaches the graph.** TypeScript recorded edges
  pointing at placeholder ids, and `dispatch` stamped them
  `Resolution::Resolved` on the way out — a name sitting where a symbol
  belongs, marked as a fact a gate may act on. `blast-radius` counted those
  names as dependent symbols, and resolution coverage counted them as resolved.

  ```
  AccessesProperty -> __UNRESOLVED_LOCAL_TYPE__|Unknowable   [resolved]
  ```

  The cause was not a missing drop. TypeScript minted its own placeholder ids
  with a private `__UNRESOLVED_LOCAL_TYPE__` prefix, structurally identical to
  the shared `unresolved_local_type` but recognised by nothing: `is_placeholder`
  — the check every layer above uses to tell a bound target from an unbound one
  — returned `false` for every one of them, so they passed every guard in the
  codebase. The other six Tier 1 adapters use the shared vocabulary and none of
  them leaked.

  TypeScript now mints the shared ids, and `dispatch` drops any surviving
  placeholder before stamping. Enforcing it centrally rather than in each
  adapter is the point: seven adapters remembering is seven chances to forget,
  and the one that forgot was the oldest and most used.

  `fixtures/adapter-ts` loses seven such edges from its golden, each of which
  had been recorded as `"resolution": "resolved"`. Four were framework and
  cyclic re-export imports (`.astro`, `.vue`, a `./a` ↔ `./b` cycle) and two
  were local types. Those references still do not resolve — this stops them
  being *reported* as resolved, which is a different and smaller claim than
  fixing them.

- **No repository could commit a rules file, so `check` had never gated
  anything anywhere.** Rules were specified to live at
  `.openinvar/rules.toml` — inside the directory every repository gitignores,
  because the graph database is there too. The file holding a repository's
  versioned constraints could not be versioned. This repository's own workflow
  said as much in a comment: *"This repository has no .openinvar/rules.toml
  yet, so there is nothing to violate."* That was true of every user as well.

  Rules now live in `openinvar.toml` at the repository root:

  ```
  openinvar.toml      # rules and audit suppressions — committed
  .openinvar/
    graph.db          # generated — gitignored
  ```

  Audit suppressions move into a `[suppress]` table of the same file, for the
  same reason: suppressing a finding is meant to be a documented act, and the
  record of what a repository had chosen to ignore previously lived where
  nobody could see it.

  ```toml
  [suppress]
  "test-tampering-1a2b3c4d" = "the test was rewritten when the API changed"
  ```

  Both old locations are still read for one release, with a deprecation notice
  naming the new one. Dropping them silently on upgrade would be a gate that
  stopped enforcing without saying so.

- **The shipped git and agent hooks would have gone quiet on an upgraded
  repository.** Each gated on `[ -f "$root/.openinvar/rules.toml" ] || exit 0`,
  so a repository that moved its rules to the root would have had its
  pre-commit and stop hooks exit 0 and enforce nothing. These hooks fail open
  by design, which makes this the failure nobody notices. They now look in both
  places through one `openinvar_rules_file` helper.

- **A mistyped section in a rules file parsed as an empty one.** `[[rules]]`
  instead of `[[rule]]` produced zero rules and a clean run; `[supress]` with
  one `p` silenced nothing. The parser now rejects unknown top-level keys,
  which is what the module's own doc comment already promised: *"Everything
  fails at parse time or not at all."*

### Added

- **`openinvar report --require-rules`**, matching the flag `check` already
  had. The GitHub Action runs `report`, so without it the action had no way to
  insist a rules file exists — and a report that comes back clean having
  enforced nothing is the one outcome a gate must never produce quietly. The
  action exposes it as a `require-rules` input.

- **This repository now gates its own CI on its own rules.** `openinvar.toml`
  pins the two ends of the crate dependency direction — nothing may reach back
  up into the binary, and the language adapters may not know how anything is
  persisted — plus an advisory fan-in threshold. The workflow runs with
  `require-rules` and `fail-on-violation` both on.

  The fan-in threshold is set at 180, above this repository's real maximum. The
  three highest are `RelationshipKind` (169), `InvarGraph` (105) and
  `node_text` (104), each a hub because it is supposed to be one. A threshold
  below those would fire on all three from the first run, and a rule violated
  on day one is a rule somebody raises until it means nothing.

### Changed

- **One released binary per platform, carrying every language.** `default` is
  now `full`. The per-language Cargo features are unchanged and still work —
  `cargo install openinvar-cli --no-default-features --features python` builds
  a Python-only binary, and the per-language CI job still builds each language
  alone. Only the released artifact collapsed. A new `tier1` feature names the
  five resolved languages together.

  Sizes on one machine with `--release`: the released binary is 30.3 MB; a
  `tier1` source build is 22.5 MB. Nobody chooses a tool on 7.8 MB, and the
  split was costing more than it saved.

### Fixed

- **The released binary never contained what the README said it did.**
  `release.yml` ran `cargo build --locked --release -p openinvar-cli` with no
  `--features`, so every published binary since v0.1.1 carried the Tier 1 set.
  The README stated that `full` "is what the releases publish", and the
  resolution-coverage figure it led with — 99.7% — was `full`'s. No released
  binary has ever reported that number; run on this repository, the binary a
  user actually downloaded reported **100.0%**, because it does not carry the
  Tier 2 languages and never opens their files.

  Both figures were individually true and the pairing was documented, so
  nothing looked wrong. The defect was that the two halves described different
  binaries, and the one being described was not the one being shipped.

  Making `default` mean `full` closes it from the other side: the published
  figure is now the figure your binary reports. Measured today on this
  repository, 99.7% — 4,865 of 4,881 edges, with the sixteen structural ones
  named per language as before.

  `crates/openinvar-lang/tests/default_build_languages.rs` pins the released
  language set so the gap cannot reopen silently, and `release.yml` carries a
  comment saying why that build line has no `--features` flag.

- **The store is SQLite, and the build no longer needs a pinned compiler.**
  One change, two problems: `librocksdb-sys` vendored a C++ database that does
  not compile on GCC 14 or newer, so every build was pinned to `gcc-13` — a
  workaround that lived in the CI workflows and nowhere a contributor would
  look. `cargo build --release`, exactly as the README documents it, now works
  on a current toolchain with nothing set in the environment. A CI job builds
  with the newest GCC the runner offers, and with no `CC`/`CXX` at all, so it
  cannot regress silently.

  | | Before | After |
  |---|---|---|
  | `--features typescript` | 21.2 MB | **14.9 MB** |
  | `default` | 28.8 MB | **22.5 MB** |
  | `full` | 36.6 MB | **30.3 MB** |

  About 6.3 MB of every binary was a key-value store used as a two-key
  dictionary.

  **The snapshot encoder was not touched.** `GraphSnapshot::from_graph` still
  emits the same hand-written length-prefixed binary blob, byte for byte;
  SQLite holds it in a `BLOB` where RocksDB held it under a key. The conformance
  suites — `tests/revisions.rs` and `tests/cache.rs` — pass **unmodified**,
  which is the evidence the port preserved the contract rather than the tests
  being taught to accept it.

  Two tables replace two column families: `graph` holds the single working
  graph, `revisions` holds snapshots keyed by revision with a `sequence` and an
  index on it. `PRAGMA user_version` replaces the layout-version key.

  **Every query that feeds output carries an explicit `ORDER BY`.** RocksDB's
  prefix iterator was implicitly ordered, so `list_revisions` was *accidentally*
  stable; SQLite guarantees no row order without one. That is this project's
  determinism rule in its SQL form, and `tests/sqlite_layout.rs` asserts the
  order holds across repeated reopens and a mid-list rewrite. Two consecutive
  `analyze --json` runs remain byte-identical.

  `journal_mode=WAL` and `synchronous=NORMAL`, because `watch` holding the
  store open while a hook invocation opens it again is a real scenario and the
  default rollback journal makes a reader and a writer exclude each other.

- **The graph database moved to `.openinvar/graph.db` — a file, not a
  directory.** There is no migration, deliberately: the graph is derived data
  that rebuilds in about a quarter of a second, so converting a cache would be
  wasted work. A store directory left by an earlier release is **reported, not
  deleted** — a read command names it and says to run `openinvar analyze`, and
  `analyze` itself notes once that the old directory is now dead weight and can
  be removed. Removing it stays the user's decision.

  `.graphyn/db` is recognised too, so someone upgrading from a release made
  before the rename gets the same message rather than a bare "no graph found".

### Fixed

- **The agent hooks stopped seeing the graph after the store moved, and would
  have failed silently.** `openinvar_has_graph` tested for a *directory*. These
  hooks fail open by design, so the pre-edit hook would have gone quiet rather
  than gone wrong — the failure nobody notices. Caught by
  `the_pre_edit_hook_reports_the_same_counts_the_cli_does`, which exists for
  exactly this.

- **`docs/architecture.md` claimed the store serialized with `serde_json`.** It
  never did, at any point. The encoder has always been hand-written and binary.
  That is the kind of wrong that costs a day when somebody plans a change
  around it, so the document now describes the real format.

### Changed

- **The project is renamed from `graphyn` to OpenInvar, and repositioned.**
  The tagline is now "Deterministic integrity checks for AI-written code" —
  structural violations, test tampering, contract erosion, with no model in
  the loop. `audit` and `check` lead the README as the two commands that gate;
  `query`, `impact` and `context` move to an "it also answers graph questions"
  section near the bottom. Those commands are unchanged and still supported —
  the demotion is about what the project leads with, not what it does.

  The renames, in full:

  | From | To |
  |---|---|
  | binary `graphyn` | `openinvar` |
  | `graphyn-core`, `-lang`, `-store`, `-mcp`, `-cli` | `openinvar-core`, `-lang`, `-store`, `-mcp`, `-cli` |
  | `graphyn_core::` and friends | `openinvar_core::` and friends |
  | `GraphynGraph`, `GraphynError`, `GraphynMcpServer` | `InvarGraph`, `InvarError`, `InvarMcpServer` |
  | `.graphyn/` | `.openinvar/` |
  | `GRAPHYN_*` environment variables | `OPENINVAR_*` |
  | `.github/workflows/graphyn.yml` | `openinvar.yml` |
  | hook scripts `graphyn-*.sh` | `openinvar-*.sh` |

  There is no short alias: the command is `openinvar`, not `invar`.

  The repository was renamed in place, so its history, issues and pull requests
  are intact and GitHub redirects the old URL. Existing `.graphyn/` directories
  are not migrated — the graph is derived data, so re-run `openinvar analyze`
  and delete the old directory.

  crates.io keywords drop `blast-radius` and `dependency-graph` for `ci`,
  `code-review`, `static-analysis` and `agents`, which is where the project now
  claims to belong.

  No analysis behaviour changed: the golden snapshots are byte-identical across
  this change, which is the evidence that it is a rename and nothing more.

- **The README is repositioned around enforcement rather than exploration.**
  Written before the rename, when the tagline read "Deterministic guardrails
  for agentic code changes"; the entry above supersedes that wording. The
  three questions are framed around the agent that is about to write: what
  breaks, which tests cover it, and does this violate a rule the repository
  wrote down.

  Determinism leads "Why Graphyn" with the reason it matters rather than as one
  bullet among five: you cannot gate CI on an opinion. A check that returns a
  different answer on a re-run is one nobody can act on, which is why every
  probabilistic tool can advise but not block.

  Resolution coverage is published prominently, with the measured figure for
  this repository — 99.7%, 4,844 of 4,860 edges, and the sixteen structural
  ones named per language rather than absorbed into a headline. The default
  binary reports 100.0% for the same repository, and the README says why that
  is the less informative number: it does not resolve more, it does not look at
  the Tier 2 files at all.

  The rules file format is documented for the first time — it had only ever
  appeared inside example output. Every kind, its fields, when it fires, and
  why severity defaults to `error`.

  Tier 2 now states plainly what it does and does not give you, and that gates
  do not fire on structural regions — not "are discouraged from", but do not.

### Fixed

- **A moved symbol was reported in a pull request comment as `name` → `name`.**
  The markdown renderer printed only the before and after names, dropping both
  the kind of continuity and the files involved — so a symbol that kept its
  name and changed file rendered as an arrow between two identical values,
  which is a row that costs a reader their attention and returns nothing. The
  terminal renderer for the same delta had always named the files.

  Each of the three kinds now renders what that kind actually tells you: a move
  names both files without repeating the name, a rename shows both names and
  the one file it happened in, and a renamed-and-moved symbol — the weakest
  evidence of the three, and so the one most likely to be checked by hand —
  shows all four values.

  The same list was also the only one in the report that truncated silently. It
  stopped at ten and said nothing, while every other list said how much it left
  out. It now says so too.

  Found by running the action against this repository's own rename pull
  request, where it produced 1,991 rows of the form `` `banner` → `banner` ``.

- **The README claimed a default binary carries every supported language.** It
  carries Tier 1 only; `full` is the everything binary. The size figures beside
  the claim were stale as well: measured now at 21.2 MB for a TypeScript-only
  build, 28.8 MB for `default`, and 36.6 MB for `full`.

- **A paragraph in the Tiers section appeared twice**, verbatim.

### Added

- **`graphyn context`** — a minimal working set for orienting on a symbol or a
  change: the subject, what it depends on, what depends on it, and a signature
  for each rather than a file body. `--budget` caps the estimated cost, drops
  the outermost hops first, and reports how many symbols it omitted rather than
  truncating silently.

  **The measured comparison, which the plan made this feature's entire
  justification, is weaker than the headline ratio suggests, and that is worth
  saying plainly.** Orienting on `RepoIR` in this repository costs ~420
  estimated tokens against ~57,900 for reading those thirty files whole — 138x.
  But an agent does not read thirty files whole; it runs a text search. Against
  `rg -l`, which answers the same "which files touch this" question, the saving
  is **1.6x**. Against `rg -n`, about 10x.

  And the part a text search genuinely cannot produce — the signature
  skeleton — is delivered for only 1 of those 31 entries. Inbound edges to a
  widely-used type are dominated by imports, which every adapter attributes to
  the file's synthetic module symbol rather than to the function that uses it,
  so "depended on by" is largely a list of files. Measured across six symbols:
  1 to 3 signatures per 5 to 50 entries, and greater depth does not help.

  The feature is therefore useful for a narrow neighbourhood where signatures
  land, and thin for a widely-imported type. Shipped with the number stated
  rather than dressed up, because the plan asked for a measurement and a weak
  measurement reported honestly is worth more than a strong one that does not
  survive being checked. Improving it means attributing import edges to the
  symbol that uses the import rather than to the module — a resolution change,
  not a presentation one.

  Token figures are byte-based estimates and say so wherever they appear.
  Graphyn vendors no tokenizer: one is model-specific, and a figure that moved
  with somebody's model would not be reproducible.


- **`graphyn audit`**, wired into the hooks and the Action. Detects changes that
  look like they were made to pass a check rather than to work.

  The output is built to be checked rather than believed, because a finding is
  an accusation. Each one carries its evidence and the id you would write down
  to suppress it — printing that id is the difference between suppression being
  a documented act and something people do by deleting the check. Suppressed
  findings are shown; a suppression matching nothing is reported as stale.

  Detectors that did not run are named with their reasons, every run. An absent
  check reads as a passing one otherwise, which is the same unearned pass the
  rest of this release exists to refuse — and a clean audit says in as many
  words that it is not a clean bill of health, since only the detectors listed
  ran and only on files a Tier 1 adapter resolved.

  A `Stop` hook and the git pre-commit hook now run it, and the Action folds an
  audit finding into the same pull request comment under its own heading rather
  than leaving it in a job log. Only an error-severity finding blocks, so the
  two high-confidence detectors gate and `dead-on-arrival` stays advisory;
  `GRAPHYN_AUDIT_SEVERITY=warn` or `audit-severity: warn` holds on everything.
  An audit that could not run never blocks — trapping an agent over a missing
  snapshot would leave it in a loop it has no way out of.


- **Three audit detectors**, and three held back with their reasons recorded.

  `test-tampering` reports a test that **stopped covering** a symbol which
  changed in the same diff — either the test went, or its references to the
  symbol did. This is narrower than the obvious rule, and deliberately: "a test
  file changed alongside the code it covers" is what a developer does on almost
  every honest pull request, and a detector firing there is switched off in a
  week. Coverage vanishing exactly where behaviour changed is hard to do by
  accident.

  `contract-erosion` reports a symbol removed while a surviving caller still
  referred to it — the callee deleted instead of the caller fixed. Resolved
  evidence only, and a referrer that was removed in the same change does not
  count: that deletion is coherent. Visibility is not modelled in the IR, so
  every surviving resolved reference is treated as a contract, which is the
  conservative reading.

  `dead-on-arrival` reports a new symbol that only tests refer to. Requiring a
  test edge is what keeps it precise: a genuinely new entry point has no
  inbound edges at all and is not reported.

  Held back rather than weakened, because a gate that cries wolf is disabled in
  week two. **`assertion-removal`** has no signal — assertion calls are not in
  the graph at all, since Rust's assert macros are token trees that are never
  expanded and framework calls resolve to nothing, so a net assertion count
  cannot be computed without parsing source text as a second source of truth.
  **`scope-creep`** has no denominator: Graphyn is given a repository and a
  revision range, never an intended scope. **`special-casing`** needs
  branch-level analysis, and the graph models references between symbols rather
  than control flow inside one. Each reason is carried in the code so that an
  absent detector is not mistaken for one that found nothing.


- **An audit framework**: findings, severity, confidence, stable ids and
  suppression via `.graphyn/audit-ignore`. The detectors themselves are a
  separate change; this is the part that decides what a detector is allowed to
  say.

  An audit finding is unlike every other finding here. The others describe
  code; this one describes conduct — it says a change looks like it was made to
  pass a check rather than to work. That is an accusation, and it will
  sometimes be levelled at someone who did nothing wrong. Three properties
  follow.

  **Detectors cannot run on evidence they cannot stand behind.** Only files a
  Tier 1 adapter resolved are in scope, and the framework drops an
  out-of-scope finding even if a detector forgot to check — accusing someone on
  the strength of a name matched inside one file must not be one mistake away.

  **A finding keeps its identity across runs.** Ids are derived from the
  detector and the symbols the finding is about, never from a line or a file
  offset, so an unrelated edit upstream does not read as a new accusation and a
  suppression written today still matches after a refactor. The hash is FNV-1a
  written out rather than `DefaultHasher`, which is explicitly unstable across
  Rust releases and would silently invalidate every suppression in a repository
  on a toolchain bump.

  **A suppressed finding is reported as suppressed, never omitted**, and a
  suppression matching nothing is reported as stale. A gate that silently drops
  what it was told to ignore is precisely the thing this feature exists to
  catch, and dead configuration gives false comfort to whoever reads the file
  next.

  Suppressions are ids, not globs. A pattern would let one line silence a whole
  detector, and a suppression that broad is indistinguishable from turning the
  check off — a decision that belongs on the command line, not buried in a file
  nobody re-reads.


- **`graphyn tests <symbol> | --diff`.** Which tests exercise a symbol, or
  everything a change touched, so a verify loop can run those instead of the
  whole suite or nothing at all.

  This is the one query whose answer licenses an *omission*: naming a subset is
  a claim that the tests left out cannot fail, and Graphyn's graph is knowingly
  incomplete — structural regions record no cross-file references, and test
  detection is by file convention. So the selection and the confidence in it
  are separate, and the confidence is carried by the exit status rather than
  only printed: 0 when every changed symbol is reached by a resolved test edge,
  3 when something could be missing, 2 when the question could not be answered.

  Tests the change itself modified are reported apart from coverage and do not
  mark anything covered. A diff that edits a function and its only test is
  exactly where a reviewer most needs to be told something is missing, and
  letting the modified test silence that would report full coverage instead.
  This list is also what the next change reads.

  Coverage reaches through intermediates — a test calling a wrapper that calls
  the changed function covers it — bounded by `--depth`. `tests` edges are not
  followed during that walk: they are the answer, not a path through the graph,
  and following them would let one test's coverage make another test look like
  a caller.


- **A `tests` relationship kind, and per-language test detection.** Every
  language now recognises its own test files by the rule its own tooling
  already uses to find them — `_test.go`, `*.test.*`/`*.spec.*`, `test_*.py`
  and `conftest.py`, a Rust crate's `tests/`, `src/test/` for Java, `_spec.rb`,
  `*Tests.cs` — and a test's references into non-test code are restated as
  `tests` edges. `--kind tests` then answers "what covers this" as a filter
  over the graph rather than a second traversal with its own rules.

  Derived, not parsed. A test already records what it calls, instantiates and
  takes as a type; the edges that leave the test file are the code it
  exercises. Three things this deliberately does not do: it adds rather than
  replaces, so a query for callers still finds tests; it ignores references
  between two test files, since a helper exercising a helper is not coverage
  and counting it would make every test appear to cover the whole suite; and it
  carries the underlying edge's resolution rather than asserting its own, so a
  Tier 2 language does not become gate-safe by passing through it. External
  packages are excluded — a test importing `std` does not test `std`.

  The blind spot, stated rather than guessed at: detection is by file
  convention, so a Rust `#[cfg(test)] mod tests` inside an ordinary source file
  is not recognised. The symbols inside such a module are not indexed at all
  today, so recognising it would name a test whose contents Graphyn cannot see.
  Integration tests under `tests/` are covered; on this repository that is 416
  test edges across 83 files.

### Fixed

- **The README claimed no adapter emits `calls` or `instantiates`.** Both have
  been emitted for some time — `adapter-go` alone produces two and five of them
  — so the note understated what the tool does. Corrected while documenting the
  new kind rather than left to contradict the list beside it.


- **Agent hooks.** `agent-configs/hooks/` ships Claude Code hooks and
  agent-agnostic git hooks. MCP is pull-only — the agent has to decide to ask —
  so the graph now reaches it at the moment of the edit instead.

  `PreToolUse` puts the target file's blast radius into context before a write.
  `PostToolUse` reports references the edit broke. `Stop` and `pre-commit` run
  `graphyn check --diff-only` and feed the specific failing rule back. A
  `post-commit` hook records the new `HEAD` snapshot, without which the
  pre-commit baseline goes stale on the next commit and the gate stops firing.

  Only one condition blocks: a rule violated on resolved evidence. A missing
  binary, a missing graph, a stale snapshot, a timeout or an unreadable rules
  file all let the change through and say on stderr that nothing was checked.
  Trapping an agent, or rejecting a colleague's commit, because a tool was
  misconfigured is worse than not running — but a gate that silently stops
  working is worse than both.

  Every call is wrapped in a timeout, falling back to a background process and
  a poll where `timeout(1)` does not exist. The full pre-edit hook runs in
  ~66 ms against this repository's most-referenced file.

- **`graphyn impact <file>`.** What depends on one file. A hook fires knowing
  only a path, with no symbol to ask about yet, so `query blast-radius` cannot
  answer it and guessing a symbol from the filename would answer a different
  question confidently. A path the graph does not track is reported as having
  no dependents rather than erroring, and an empty answer is qualified by
  whether the graph could have seen a dependent at all.

- **`graphyn check --diff-only`.** Shorthand for `--base HEAD --head worktree`,
  which is what a pre-commit hook wants.
- **Two MCP tools: `graph_diff` and `check_rules`.** The enforcement half of
  Graphyn, reachable by an agent rather than only by a command line.
  `graph_diff` reports what a change broke; `check_rules` evaluates
  `.graphyn/rules.toml` and says which constraints it violates.

  Six tools total, which is a ceiling rather than a coincidence — a large tool
  surface degrades an agent's ability to pick the right one, so a question an
  existing tool already answers does not get its own. The edge-kind and
  resolution filters the plan also called for were already present on the three
  query tools and were left alone.

  Both read snapshots and never analyze. Without one they name the command that
  records it rather than analyzing on the fly: an answer that depended on
  whatever was on disk at the moment of the call would not be reproducible, and
  reproducibility is the property the whole product rests on.

  `check_rules` reports the same four outcomes the command does, and takes more
  care with them, not less. A person reading a terminal notices an empty
  section; a model reading "no violations" acts on it. So the sentence saying
  every rule passed is emitted only when every rule was examined in full, and a
  repository with no rules file is told that nothing was enforced rather than
  that nothing was wrong.

### Changed

- **Revision naming moved from the CLI to `graphyn-store`.** The store keys
  snapshots by these names and two front ends now resolve them. Two copies of
  the rule would drift, and the failure when they do is a snapshot written under
  one name and looked for under another.
- **A GitHub Action**, and `graphyn report` behind it. The action analyzes the
  base and head commits, compares them, evaluates `.graphyn/rules.toml`, and
  posts one pull request comment — updating it on each push rather than adding
  another, because a pull request with nine identical bot comments is one where
  nobody reads the tenth.

  The comment is a product surface, not a serialisation of internal state. The
  verdict is its first line, so a reader who stops there still has the answer.
  Findings come before counts: a broken reference is what the reader came for
  and "14 symbols added" is not. Uncertainty is never dropped for tidiness — a
  rule that could not be decided is named in the comment, not only in the exit
  status, and never fails the job.

  `graphyn report` produces that markdown as one command rather than two.
  Concatenating `diff` and `check` would give the reader two headings, two
  verdicts, and no statement of which one decides the merge. Its exit status
  matches `check`: 0 clean, 1 violated, 2 could not run.

  `version: source` builds from the checked-out tree instead of downloading a
  release, which is how this repository runs the action against its own pull
  requests. An action nobody has executed is not a tested action, and running
  it caught the first two things wrong with it.

  The comment read "No problems found" on a repository with no rules file,
  which is the unearned pass `check` already refuses to report. The report now
  states that no rule was enforced, and the headline claims only what the diff
  supports.

  And the comparison ran against the tip of the base branch rather than the
  merge base, so everything merged into the base since the branch diverged read
  as removed by the change. On Graphyn's own pull request that produced a
  comment claiming a file the branch never touched had lost most of its
  symbols. The range is now the branch's own work and nothing else.


- **`graphyn check`** — enforce the rules in `.graphyn/rules.toml`. Reads the
  rules, evaluates them against the stored graph, and reports every rule with
  its verdict. `--base` and `--head` supply a change so that change-sensitive
  rules can run; `--rules` points at a different file; `--json` emits the whole
  result under a versioned schema.

  Exit status is the contract, and it distinguishes three things a gate must
  not confuse: `0` when nothing was violated, `1` when a rule was broken on
  resolved evidence, `2` when the check could not run at all — an unreadable
  rules file, no graph, a missing snapshot. A pipeline can tell "your change is
  bad" from "this tool is misconfigured" instead of reading both as a veto.

  Nothing that did not pass is reported as a pass. A rule too weakly resolved
  to judge is listed as undecided, a rule needing a change that was not given
  is listed as skipped, and neither fails the build — but the closing summary
  names them, and "All rules satisfied" is printed only when every rule was
  examined in full and none broke. A warn-severity breach exits 0 and still
  says a rule was violated.

  A missing rules file exits 0 and says plainly that nothing was enforced;
  `--require-rules` makes it a failure, for CI, where a rules file that has
  gone missing would otherwise pass silently.

### Fixed

- **`no-field-removal` attributed no fields at all against a real repository.**
  Ownership was recovered from the container's line range, but adapters
  disagree about what that range means — TypeScript records only the
  declaration line, leaving `line_end` equal to `line_start`, so nothing was
  ever contained. Exact containment is still preferred where the range is real,
  and the innermost enclosing container wins so a nested class keeps its own
  fields; otherwise the owner is the nearest container declared above. Found by
  running the rule against a fixture rather than by a synthetic test, all of
  which had passed.

- **`max-fan-in` counted third-party packages.** Every repository points at its
  serialization library hundreds of times, so the rule flagged external
  packages ahead of anything in the codebase and its threshold would have been
  raised until it meant nothing. External package nodes are no longer counted;
  a violation with no known location now names the symbol id rather than
  rendering as `:0`.


- **Rules evaluated against a graph**, with three outcomes rather than two.
  `graphyn_core::rule_eval::evaluate` runs the rules from `.graphyn/rules.toml`
  over a graph, and a delta where one is supplied.

  A rule is reported satisfied only when every edge in its scope was resolved.
  Where weaker edges could hide a violation the verdict is *inconclusive*,
  carrying the count of edges that prevented an answer. A forbidding rule is a
  claim about absence, and "no violation found" is not "no violation exists" —
  reporting a pass on structural evidence would report a conclusion the
  analysis never reached. Inconclusive does not fail a gate, which is how a
  Tier 2 language fails open rather than passing quietly.

  Violations are only ever raised on resolved evidence, so a reported violation
  is a fact rather than a suspicion; a violation found on strong evidence is not
  softened by weak evidence elsewhere in the same scope.

  `forbid-dependency` covers imports and re-exports, `forbid-reference` every
  edge kind, so "you may call into this, but not import it" is expressible.
  `max-fan-in` counts resolved inbound edges, and raises uncertainty only when
  the unresolved ones could actually carry a symbol past the threshold.
  `no-field-removal` matches fields by declaration range rather than by name
  convention, and stays quiet when the owning symbol was deleted outright —
  that removal is already reported, and repeating it per field would bury it.
  A rule needing a delta without one is skipped, never satisfied.


- **Findings derived from a diff.** `graphyn diff` reports broken references,
  orphaned symbols, removed API surface and changed signatures, alongside the
  raw counts, with the same set in `--json`. Each finding carries its own
  resolution — the weakest among the evidence that produced it, since a finding
  is only as trustworthy as the shakiest step in the reasoning behind it — and
  `graphyn_core::findings` exposes the gate-safe subset directly, which is what
  the enforcement commands will read.

  Coverage is measured over the changed files rather than the repository. A
  change confined to one badly resolved file is not made trustworthy by the
  rest of the repository resolving well.

  Two things this deliberately does not do. A symbol removed together with its
  only caller is not breakage — reporting a clean deletion would bury the
  reports that matter. And visibility is not modelled anywhere in the IR, so
  "public method removed" cannot be distinguished from "private method
  removed": the finding names every API-shaped symbol removed, and says so.
- **`.graphyn/rules.toml`, parsed and validated.** The file a repository uses
  to state its own constraints: `forbid-dependency`, `forbid-reference`,
  `no-field-removal` and `max-fan-in`, each with a `severity` of `warn` or
  `error`. Parsing and validation only — evaluating a rule against a graph is
  a separate change.

  Everything that can be wrong is wrong at parse time. An unknown kind, a glob
  that does not compile, a required field left out, a `max-fan-in` threshold of
  zero, two rules under one name: each is refused when the file is read, with
  the rule named and the valid kinds listed. A typo would otherwise sit
  silently in a repository until the day it was supposed to catch something,
  and a gate that quietly enforces four of five rules reports a pass it has not
  earned.

  Severity defaults to `error`. A rule written without one is a rule someone
  means to enforce, and defaulting to advisory would make every unannotated
  rule silent.

  Rules are partitioned by whether they need a delta — only `no-field-removal`
  does — so a caller holding a single graph skips the rest rather than
  reporting them as passing.


- **`graphyn diff`.** Compares two recorded revisions and reports what changed:
  symbols added, removed, renamed or moved, signatures changed, and edges added
  or removed. `--base` defaults to `HEAD`, `--head` to `worktree`, and `--json`
  emits a versioned document — schema 1 from its first release, since hooks, CI
  and the MCP tools read it.

  It never analyses. Both sides come from snapshots `analyze --snapshot`
  already recorded, so the same pair of revisions always produces the same
  answer and the command costs a read rather than a re-analysis. A revision
  with no snapshot is an error naming the command that would record it; quietly
  analysing one side would make the result depend on the working tree at the
  moment the command happened to run.

  Every diff reports how much of itself is gate-safe, not only when the news is
  bad. A figure that appears sometimes is one nobody learns to read, and a diff
  made entirely of structural edges says so explicitly.


- **Graph delta.** `graphyn_core::delta::compute` compares two graphs and
  reports added, removed and continued symbols, signature changes, and added
  and removed edges. Library only; the `diff` command is a separate change.

  A symbol's id embeds its file and its name, so a rename or a move produces a
  different id and a naive comparison reads it as a delete plus an unrelated
  add — the loudest possible description of the smallest possible change, and
  one that would make a later audit detector count a rename as contract
  erosion. So a removed symbol and an added one are paired into a *continuity*
  where the evidence supports it, and the pairing records whether the name, the
  file, or both moved.

  What counts as evidence is deliberately narrow. Kind and language must always
  match, because a function replaced by a class of the same name is a real
  change rather than one symbol that moved. A rename in place needs a matching
  signature or the same starting line; a symbol that changed both name and file
  needs a matching signature, since nothing else distinguishes it from an
  unrelated addition. An absent or empty signature is treated as no evidence at
  all — every symbol lacking one would otherwise match every other. Where the
  evidence is not there, the change stays a removal and an addition, which is
  the honest description of "these may be the same symbol and nothing here can
  tell".

  Pairing is greedy over a fully ordered candidate list rather than an optimal
  assignment. Optimal matching would be more accurate in contrived cases and
  less predictable in every case, and predictability is what is being bought:
  the same two graphs must always produce the same delta. Every field of the
  result is ordered, and a test runs the same comparison repeatedly to assert
  it.

  `has_resolved_changes` reports whether a delta touches anything a gate may
  act on, so a change made entirely of structural edges — a Tier 2 repository —
  is visibly not actionable rather than silently passing.

- **Graph snapshots keyed by revision.** `analyze --snapshot <rev>` records the
  analysis under a revision alongside the working graph, which is what `diff`
  will compare. `--keep-snapshots N` (default 10) drops the oldest beyond N.

  A commit, branch or tag is resolved through git to its SHA and stored under
  that, because `HEAD` means something different tomorrow and a snapshot named
  for a moving label stops describing what it holds. `worktree` is the one name
  kept literally: it is the working tree including uncommitted edits, so it has
  no commit to resolve to. A revision git cannot resolve is an error rather
  than a snapshot under that name — a typo becoming its own snapshot would make
  `diff` compare against an empty graph and report that everything was added,
  which is a confidently wrong answer where an error is the honest one.

  Snapshots live in their own RocksDB column family rather than under a key
  prefix. The working graph is read by every query and rewritten by every
  analyse; revisions are written once and read only by `diff`. Keeping them
  apart means retention cannot iterate over or delete the working graph, and a
  layout change can drop the revisions alone. That layout carries its own
  version, separate from the snapshot payload's: a mismatch discards the family
  and reindexes, because a stale index pointing at snapshots that no longer
  parse is the silent corruption this is meant to prevent, and a revision
  snapshot is only ever a cache of something git can reproduce.

  Retention orders revisions by a counter rather than a wall clock. Two
  snapshots written inside one clock tick would be unordered and a clock
  adjustment would reorder history; a counter is monotonic by construction,
  which is all retention needs.

- **Ruby and C# as Tier 2 languages** (`--features ruby`, `--features csharp`,
  neither in `default`). Each is a module, a feature flag and a spec — no
  parser, extractor, resolver or query file, because the analyzer runs on the
  `tags.scm` the grammar already ships. That is the tier working as designed.

  Fewer languages than the batch this was scoped for, and the reason is worth
  recording: the constraint is not Graphyn's architecture but the grammar
  crates. A candidate needs one that both works against the `tree-sitter`
  version pinned here and ships a tags query. Scala and Lua pin an incompatible
  `tree-sitter`; PHP and Swift pull a second copy of it into the build; Kotlin,
  SQL and Bash ship no tags query at all — their `TAGS_QUERY` is commented out
  upstream. Each was tried and rejected on evidence rather than assumed, and a
  `tree-sitter` upgrade would unblock most of them.

  A Tier 2 language is reported as such by `graphyn status`, resolves nothing
  across files, and every edge it produces is `structural`, so the new
  `--min-confidence resolved` threshold excludes it and later gates fail open
  on it. Five tests per language assert exactly that, half of them about what
  Tier 2 cannot do.

- **`graphyn status` reports resolution coverage, per language.** Every edge
  already recorded whether it was bound through imports, aliases and declared
  types or matched by name inside one file; nothing reported the ratio. That
  ratio is what makes an enforcing tool trustworthy — "nothing broke" means
  something very different at 98% resolved than at 40%, and a gate that cannot
  tell them apart will eventually pass a change it should have stopped.
  Reported per language as well as overall, because coverage is rarely uniform
  and one badly resolved language otherwise hides inside the total. A file with
  no relationships reports no coverage rather than 0% or 100%, both of which
  would state something untrue about it. Where any structural edge exists the
  known blind regions are named: Tier 2 languages, C++ template instantiations,
  Rust macro bodies, cross-language imports, and chained access past the first
  receiver.

- **`--min-confidence` on `blast-radius`, `usages` and `deps`.** `resolved`
  restricts an answer to edges a gate may act on; `structural`, the default,
  keeps today's behaviour. The threshold is applied during traversal rather
  than to the result set: an edge below the floor is not followed either, since
  a symbol two hops away reached by way of a guess is itself a guess. The same
  threshold is available to the MCP query tools as `min_resolution`. On a Tier
  2 repository `--min-confidence resolved` correctly returns nothing, which is
  the honest answer to "what may I act on here".


- **A C call through a header prototype reaches the definition.** C splits a
  call across two files, and the shape is the dominant one in plain C: the
  caller includes a header that *declares* the function, while the definition
  lives in a `.c` file the caller never sees. Callers now attach to the
  definition. That direction is the point — the graph answers "what breaks if I
  change this", and a caller attached to the declaration would leave
  `blast-radius` on the definition returning nothing, the exact failure call
  edges exist to prevent.

  The link is an agreement between two files rather than a name matched across
  the repository: a header declares `N`, and exactly one file both defines `N`
  and includes that header. Including the header that declares you is what a C
  build already does so the compiler can check the two agree, so the rule keys
  on a fact the language enforces rather than on a filename convention like
  `geometry.c` implementing `geometry.h`. Two definers make it ambiguous and a
  definer that does not include the header is unanchored; both record nothing
  rather than guess, and neither raises a diagnostic. A definition the caller
  can see directly still wins, so a `static` helper shadows an external
  function of the same name exactly as it does at compile time.

  A prototype is not a symbol. It names a function defined elsewhere, so
  minting a node for it would put two nodes in the graph for one function and
  make the name ambiguous to `find_symbol_id` — the ambiguity 0.2.0 spent a
  release removing. The placeholders exist only between extraction and
  resolution and are dropped before the graph is returned.


- **Call and instantiation edges for C and C++.** A bare `foo(..)` records a
  call when the name resolves to a function the file can see — its own, or one
  defined in a header it includes. `new Foo(..)` records `Instantiates`; C has
  no construction syntax at all, since `struct Foo f = {..}` is a declaration
  with an initializer rather than an expression naming a constructor. A C++
  functional cast, `Celsius(x)`, is spelled exactly like a call and calls
  nothing, so as elsewhere the resolved target's kind settles the kind.

  This is the narrowest of the five languages, and deliberately so. `ns::func()`
  and `obj->method()` record nothing: C++ methods are not symbols in this graph,
  only their class is, so an edge would either name the class — claiming the
  class was called — or match a leaf name repository-wide, which is the bug
  0.2.0 fixed elsewhere.


- **Call and instantiation edges for Go.** Go is the one language where the
  selector rule had to be inverted. `obj.method()` records no call edge in
  TypeScript, Python and Rust, but a cross-package call in Go is *always*
  written `pkg.Func(..)` — applying the same rule would have left Go with call
  edges that never cross a file boundary. So a selector call is recorded when
  its operand is one of the file's imported package names, and skipped when it
  is a value. The file's own import list is the only thing that tells the two
  apart, and it is already parsed.

  `Foo{..}`, `pkg.Foo{..}` and `&Foo{..}` are composite literals — Go's
  construction syntax — so they record `Instantiates` from the syntax alone. A
  composite literal for a slice, map or array names no symbol and records
  nothing.

  `models.UserID(42)` is a conversion spelled exactly like a call. Nothing is
  called, so as in Python the resolved target's kind settles the kind: a target
  that turns out to be a type makes the edge an instantiation. A `Calls` edge
  still only ever targets a function or a method.

  Builtins (`len`, `make`, `append`) and standard-library calls record nothing
  and raise no diagnostic, since neither names a symbol in the graph.


- **Call and instantiation edges for Rust.** Rust spells the cases differently
  and each is recorded as what it is. `Foo::new(..)` is an associated function
  call, so the edge names the *method* `Foo::new` rather than the type — `new`
  returns `Self` only by convention, and an edge to the type would leave
  `blast-radius` on the method blind to its own callers. `Foo { .. }` is Rust's
  actual construction syntax and records `Instantiates` outright. `Foo(1)` on a
  tuple struct reads as a plain call, so, as in Python, the resolved target's
  kind is what makes it an instantiation.

  `Enum::Variant(x)` is construction too, and is recorded as such: nothing ever
  calls a variant, and an edge saying otherwise would surface under
  `--kind calls` as a caller that does not exist. After both promotions a
  `Calls` edge only ever targets a function or a method — something that can
  actually run.

  A path callee resolves through the file's imports and then into the file that
  defines the type, never by matching a leaf name across the repository. A
  fully-qualified path used inline without a `use` still records no edge, which
  is the documented limit and is now pinned by a test rather than only by
  prose. `obj.method()` records nothing, `println!(..)` is a macro invocation
  rather than a call node, and prelude names bind to nothing — none of them
  raise a diagnostic, because there is nothing a user could fix.


- **Call and instantiation edges for Python.** `foo()` where `foo` was imported
  or defined in the file records a `Calls` edge to the definition, travelling
  through the same import machinery as every other reference, so a call through
  a renamed import resolves to the canonical symbol rather than the alias.

  Python spells construction and invocation identically — `Foo()` and `foo()`
  are the same node — so the split is made from the resolved target: an edge
  whose target turns out to be a class is promoted to `Instantiates`. Deciding
  it from a capitalized name would be a naming convention presented as an
  analysis.

  As in TypeScript, `obj.method()` records no call edge (it is already a
  property access on the receiver, which is the honest statement), and a callee
  that binds to nothing records nothing at all — no `print()` edge, and no
  diagnostic, because there is nothing a user could fix. A call landing on a
  third-party import is dropped for the same reason: the only id available is
  the package, and "this function calls the requests package" is a claim about
  a symbol nobody can open. The `Imports` edge already carries that dependency.


- **Per-edge resolution, and a safety verdict that depends on it.** Every
  relationship carries a `resolution` of `resolved` (bound through the file's
  imports, aliases and declared types) or `structural` (matched by name inside
  one file). A polyglot graph mixes both in one answer, so it belongs on the
  edge: `blast-radius` over a TypeScript and Java repository returns rows from
  each, and until now they read identically.

  `blast-radius` finding nothing prints "safe to modify" — a claim about the
  whole repository. It now holds that claim back when any edge in the graph is
  structural, naming the files that were analyzed within-file only, because a
  reference living in one of them would never have reached the graph. The
  result is still reported; what changes is that it is no longer presented as
  proof. Structural rows in a non-empty result are marked in both the CLI and
  the MCP context, since an agent acts on that line.

  Adapters do not set the field: dispatch stamps `Resolved` on Tier 1 output
  and structural analysis leaves the default. The default is the *weaker*
  value, so forgetting it under-claims rather than granting gate-safety.


- **Call and instantiation edges**, for TypeScript and JavaScript. `foo()`
  where `foo` arrived through an import or is defined in the file records a
  `Calls` edge to that function; `new Foo()` records `Instantiates`. Both
  travel through the existing import machinery, so a call to a function
  imported through a barrel chain resolves to the file that defines it.

  A callee that binds to nothing records **no edge at all** — not an edge
  against a placeholder. `setTimeout(...)`, a global, or a value from an
  untyped module names no symbol in the graph, and inventing one by matching
  the leaf name repository-wide is the bug 0.2.0 fixed in the Rust adapter.
  No diagnostic is raised for these, because there is nothing a user could fix.

  `obj.method()` is deliberately not a call edge. It is already recorded as a
  property access on the receiver's declared type, which is the honest
  statement; a `Calls` edge to the *type* would claim the type was called and
  would double every row for one source location.


- `graphyn analyze --json` — emits the analysis as a machine-readable document
  on stdout instead of a human summary. Carries a `schema_version` so a
  consumer can tell a shape it understands from one it does not; the version is
  present from the first release that emits JSON at all, because retrofitting
  one forces every existing consumer to handle its absence.

- `--kind` on `query blast-radius`, `usages` and `deps`, restricting a
  traversal to named relationship kinds (`imports`, `extends`, `implements`,
  `uses-type`, `accesses-property`, `re-exports`). Repeatable. MCP query tools
  take the same filter as a `kinds` argument, and every returned edge now
  reports its kind.

  Filtering happens on the edge rather than on the collected result, so an
  excluded kind also stops the walk continuing through it: asking what imports
  a symbol will not return something that merely inherits from an importer.

  An unrecognised name is rejected rather than ignored, and a filtered empty
  result is never reported as "safe to modify" — the tool searched a slice of
  the graph, so the claim would be unearned.

- Golden snapshots of the analysis over the whole fixture corpus, and a CI
  step asserting that two analyses of identical input agree byte for byte.
  Per-adapter tests each assert one property of one language, which catches the
  regression someone thought to look for and misses a refactor that quietly
  changes the graph everywhere. Regenerate with `UPDATE_GOLDEN=1 cargo test -p
  graphyn-cli --test golden_ir` and review the diff.

### Changed

- **Adding a language no longer means editing a list in three places.** The
  spec registry carried a `NUM_SPECS` constant repeating every feature so a
  fixed-size array had a length, and `adapter_group` carried one match arm per
  language to route files to it. Both drifted the first time two languages were
  added at once: the constant stopped compiling, and — worse, because it was
  silent — a language with a spec and a feature flag was never routed, so its
  files were skipped and it simply produced nothing. The registry now pushes,
  and Tier 2 routing is derived from the registry rather than restated, so a
  new structural language touches its own module and nothing else.

- **A tier model, so "supports N languages" means something.** `LanguageSpec`
  describes a language and how much of it Graphyn can resolve. Tier 1
  (`resolved`) has full import, alias and declared-type resolution and is safe
  to gate on. Tier 2 (`structural`) has symbols and intra-file references only,
  and is advisory by construction — a gate can fail open on it automatically
  rather than reporting a pass it did not earn. `graphyn status` reports the
  tier of every language a build carries.

  Consolidating the adapters removed the packaging cost of a language, not the
  engineering cost. Import resolution, alias resolution and declared-type
  binding are irreducibly per-language and cost weeks; every bug 0.2.0 fixed
  was a resolution bug, not a parse bug. Tiers are how breadth stays honest.

- **Tier 2 needs no query files written for it.** Every tree-sitter grammar
  Graphyn vendors already ships `queries/tags.scm` and exposes it as
  `TAGS_QUERY`, using a standard capture vocabulary — `@definition.*`,
  `@reference.*`, `@name` — that maps onto Graphyn's own `SymbolKind` and
  `RelationshipKind`. One generic analyzer covers every structural language.
  Adding one is a dependency, a feature flag and a spec.

- **Java, as the first Tier 2 language** (`--features java`, not in `default`).
  Its whole module is a spec: no parser, no extractor, no resolver, no queries.
  It extracts classes, interfaces and methods and links intra-file calls and
  `implements`; it resolves nothing across files, and `status` says so.

- **Ten crates became five.** The five `graphyn-adapter-*` crates and
  `graphyn-adapter-dispatch` are now one crate, `graphyn-lang`, with a module
  and a Cargo feature per language. `graphyn-core`, `graphyn-lang`,
  `graphyn-store`, `graphyn-mcp` and `graphyn-cli` remain.

  Three arguments usually justify a crate per language and none held here:
  every adapter depended on `graphyn-core` and was consumed only by dispatch,
  so they versioned in lockstep and had exactly one consumer; and the expensive
  compilation is the generated C in the upstream `tree-sitter-*` crates, which
  cargo caches regardless of how this code is laid out.

  The cost was real, though. The publish job had to push ten crates in
  dependency order, and a failure partway through leaves earlier crates on
  crates.io at a version that cannot be reused. Every new language lengthened
  that chain. Adding a language no longer touches the publish job at all.

  Under SemVer 0.x this is a permitted breaking change, and 1.0.0 is the last
  point at which the crate names are not yet a public commitment.

- The `graphyn-adapter-ts`, `-python`, `-rust`, `-go`, `-c` and `-dispatch`
  crate names are retired. A final patch release of each, carrying a
  deprecation notice pointing at `graphyn-lang`, should be published from the
  0.2.0 tag before 1.0.0 goes out. Nothing is yanked: yanking breaks existing
  lockfiles for no benefit.

- **`supported_languages()` reports what a build carries**, rather than a
  hardcoded list that happened to be accurate because only one build existed.

- Per-language features reach the binary: `cargo install graphyn-cli
  --no-default-features --features python` produces a working Python-only
  `graphyn` that indexes Python and skips other languages rather than failing
  on them. Measured on this machine, that binary is 16M against 27M for all six
  languages.

### Known limits

- **A stale reference inside an edited file is not reported as broken.** A
  reference that no longer resolves produces no edge at all in the new graph
  rather than a dangling one, so two graphs cannot distinguish "the author
  updated this caller" from "the author left a stale reference here". Broken
  references are therefore reported only where the referring file was left
  alone entirely; rename a type in a file and delete a function that same file
  still calls, and nothing is reported.

  The analyzer's own "unable to resolve" diagnostics say it directly, but they
  are neither carried on the graph nor persisted in a snapshot, so closing this
  is a format change rather than a better heuristic. Failing silent is the
  right direction meanwhile: an advisory finding on every removal whose caller
  was correctly updated is the kind of noise that gets a gate switched off. A
  test pins the gap so it stays visible.

- **Visibility is not modelled.** Nothing in the IR records whether a symbol is
  public, so a removed private helper and a removed public method are
  indistinguishable, and "visibility narrowed" cannot be detected at all.


- **Tier 2 languages resolve nothing across files.** A structural language
  records a reference only when the name is defined in the same file. A tags
  query reports that a call to `foo` happened; it does not say which `foo`, and
  matching by leaf name repository-wide is precisely the bug 0.2.0 fixed in the
  Rust adapter. "No usages found" in a Tier 2 file is a statement about that
  file.

- **Fully-qualified paths used inline are not resolved.** A type written as
  `graphyn_core::ir::RepoIR` in a signature, with no `use` bringing it into
  scope, records no edge — resolution binds names through the file's import
  table, and such a path never enters it. On this repository that accounts for
  21 of the 45 files mentioning `RepoIR`, and for the `Display` and `Error`
  references reported against `impl std::fmt::Display for …`. Pre-existing, and
  unrelated to crate-root discovery; documented here because the workspace fix
  is what made it visible.

- **Call and instantiation edges cover every Tier 1 language, at different
  depths.** C and C++ are the narrowest, and Tier 2 languages emit `Calls`
  within a file only. A `--kind calls` query against a Java repository therefore
  returns only within-file callers — and says so, naming the graph rather
  than the feature: the CLI and MCP tools report which kinds the analyzed graph
  actually contains instead of consulting a hand-maintained list of
  unimplemented kinds. That list was wrong twice inside one release, and could
  not express "call edges exist, but not for this repository's language" — which
  is exactly the statement that had to stay true as each language followed.

### Fixed

- **Two unrelated symbols sharing a line were paired as a rename.** Introduced
  and caught within this release. Pairing a removed symbol with an added one
  accepted "same file, same starting line" as evidence of a rename, and
  deleting one function while adding another in its place produces exactly
  that. `doomed` and `freshlyAdded` were reported as one renamed symbol.

  A signature is the symbol's source text, so a rename changes it and raw
  equality never matches a genuine one — which is why the position heuristic
  was there. Substituting the old name for the new one separates the two
  precisely: `class UserPayload {..}` becomes `class CustomerPayload {..}` and
  matches, while `function doomed() { return 2 }` becomes
  `function freshlyAdded() { return 2 }` and does not match a body returning 3.

  The cost is that a rename which also edits the body records no continuity.
  That is the honest answer — the evidence for "same symbol" is gone — and
  inventing a rename is the worse error, since a later audit detector reads a
  removal as contract erosion.

  Found by running `diff` against a real repository rather than by a unit test:
  the synthetic fixture gave a renamed class the same signature literal as the
  original, which no analyzer produces.

- **`graphyn-lang` did not build with only a Tier 2 language enabled.** With
  every Tier 1 language turned off, each arm of the dispatch `match` is compiled
  out and the sole remaining arm is `Vec::new()`, leaving an element type
  nothing pins down; the `java_structural` test additionally carried an unused
  import that only a java-enabled build ever compiled. `cargo build
  --no-default-features --features java` failed on both. Neither was reachable
  from `--workspace`, because java is not a default feature, so the file that
  broke was never compiled by CI.

  CI now runs clippy over `graphyn-lang` with each language feature enabled on
  its own, taking the list from the crate's own feature table so a new language
  is covered without anyone remembering to add it in two places. The
  contributing guide has asked for this check by hand since the languages
  became features; asking was not enough.

- **The snapshot format dropped the resolution of every edge.** Introduced and
  caught within this release: `GraphSnapshot` did not persist the new field, so
  a graph loaded from `.graphyn/db` — which is every query, since queries read
  the store — came back entirely `structural`. The failure was in the safe
  direction and therefore silent: nothing errored, but `blast-radius` refused
  to say "safe to modify" about a fully resolved repository, and the feature
  was useless without being visibly broken. Snapshot version is now 3; a
  version 1 or 2 snapshot reads back as `structural` and is rewritten on the
  next analyse.

- **`is_aliased_only_property` tested the wrong predicate.** It checked
  `alias.is_some()` while `partition_by_alias` had already been corrected to
  call `is_renamed` — one call site missed by commit `498333e`, duplicated
  verbatim in the CLI and the MCP server. Adapters record the local name on a
  reference whether or not it differs from the symbol's own, so an ordinary
  `import { UserPayload }` had every field it touched labelled "(aliased import
  only)". The predicate now lives in `graphyn-core` next to `is_renamed`, with
  both copies deleted.

  It also returned true for a property no edge touches, because `all` over an
  empty iterator is true — a claim about references that do not exist.

- **"imports as X" was printed for two opposite meanings.** The aliased branch
  rendered "renamed to X" and the fallback rendered "imported by X" using the
  identical phrase, so a reader could not tell which was meant — in the tool
  whose entire pitch is alias awareness. A genuine rename now reads "renamed
  to X"; a module-scope referrer, which was most of what the fallback emitted
  as "imports as module", prints nothing, since the location line already says
  where the reference is. Fixed in all three places it had been copied to.

- **One reference was reported as several dependents.** A source location
  attributed at both class and method level produces two edges differing only
  in `from`, and `from` was part of the deduplication key, so both survived. A
  64-file project reported 196 "dependents" for 38 referencing files, with the
  aliased findings below 160 rows of duplicates. `from` no longer splits rows;
  `kind` still does, because an `extends` and a field read at one line are two
  different facts. Sorting now happens before collapsing, so which row survives
  is decided by the ordering — lowest hop wins — rather than by traversal
  order.

- **The alias count counted symbols, not aliases.** `alias_chains` is keyed by
  symbol, so its length is the number of symbols that have aliases. Reported as
  "Alias chains", it read as a count of renames: a type imported under a
  different name by twelve files reported `1`. Both numbers are now reported —
  "Aliases 21 (across 19 symbols)" — and JSON gains an `aliases` field
  alongside `alias_chains` rather than redefining it, since adding a field is
  not a contract change and redefining one is.

- **Rust resolution assumed a single crate rooted at `<repo>/src/`.** In a
  Cargo workspace nothing resolved: `crates/graphyn-core/src/ir.rs` was read as
  the module `crate::crates::graphyn-core::src::ir`, so every `crate::` path
  failed and every `use graphyn_core::…` was classified as a third-party crate.
  Graphyn could not analyze itself — `query usages RepoIR` found nothing and
  `blast-radius RepoIR` reported a type referenced across the workspace as safe
  to modify. It now finds 36 usages and 42 dependents.

  The same assumption broke *nested* crates that are not workspace members,
  including this repository's own Rust fixtures, which were folded into one
  imaginary crate alongside the real code.

  Crate roots are now discovered per package — `[workspace] members` with globs
  and `exclude` honoured, `[lib]`/`[[bin]]`/`[[test]]`/`[[bench]]`/`[[example]]`
  paths read, and Cargo's conventional locations used as a fallback. Module
  paths are namespaced by the crate that owns them, so `crate::ir` inside
  `graphyn-core` and `graphyn_core::ir` written anywhere else resolve to one
  place, and intra-crate and inter-crate resolution become the same lookup
  rather than two mechanisms that have to agree. Package names normalise
  (`graphyn-core` → `graphyn_core`), `[lib] name` overrides are honoured, and
  dependency renames (`foo = { package = "bar" }`) are followed.

  A tree with no `Cargo.toml` still resolves: a `src/lib.rs` is a crate root
  whether or not a manifest sits beside it. Requiring one would have silently
  reclassified such paths as third-party rather than reporting them unresolved,
  and a wrong answer with no diagnostic is the worst available outcome.

- **Serialized output was not reproducible.** `RepoIR.language_stats` was a
  `HashMap`, and the dispatch layer built it as an ordered `BTreeMap` only to
  collect it straight back into a `HashMap`, discarding the ordering it had
  just established. `HashMap` iteration order is seeded per process, so the
  same analysis emitted its language counts in a different order on every run.
  Nothing surfaced it before because no output path serialized the field.
  Determinism is the property the graph is built to guarantee, and it has to
  hold all the way out to the bytes on stdout.

## [0.2.0] - 2026-08-16

Graphyn is no longer TypeScript-only. This release adds Python, Rust, Go and
C/C++ adapters behind a dispatch layer, and grounds property resolution in
declared types across every adapter.

### Added

- **Python adapter** (`graphyn-adapter-python`) — `.py` and `.pyi`. Relative
  imports, `__init__.py` re-export chains, star imports, `TYPE_CHECKING`
  guards, and Pydantic / Django / dataclass field attribution.
- **Rust adapter** (`graphyn-adapter-rust`) — `.rs`. Module tree from `mod`
  declarations, `use` groups and aliases, glob imports, trait impls,
  `#[derive(...)]`, and field access recovered from macro token trees.
- **Go adapter** (`graphyn-adapter-go`) — `.go`. Package imports resolved
  against `go.mod` discovered per directory, and structural interface
  satisfaction from method sets.
- **C/C++ adapter** (`graphyn-adapter-c`) — `.c` `.h` `.cpp` `.cc` `.cxx`
  `.hpp` `.hxx` `.hh`. `#include` resolution, `typedef` aliases, C++ `using`
  aliases, base classes and namespace-qualified names.
- **Dispatch layer** (`graphyn-adapter-dispatch`) — routes files to the adapter
  that owns their language, runs adapter groups in parallel, and merges the
  results into one `RepoIR`. `supported_languages()` reports what a build can
  analyse.
- Shared symbol-id and AST traversal helpers in `graphyn-core`, replacing five
  per-adapter copies.
- Adapters emit parse, resolution and skip diagnostics; pruned directories are
  reported.
- Cross-language regression suite and polyglot fixtures. Test count went from
  39 to 185, replacing 27 `assert!(true)` stubs with real coverage.

### Changed

- The CLI and MCP server now call `graphyn-adapter-dispatch` instead of
  `graphyn-adapter-ts` directly. Mixed-language repositories index in one pass.
- Dispatch groups files by a stable ordering key and sorts each group, so graph
  construction order is reproducible across runs. `HashMap` iteration order had
  made two runs over identical input produce differently-ordered output.
- AST traversal is iterative and depth-bounded. Recursion over deeply nested
  input could overflow a rayon worker's stack and abort the process.
- Unresolved edges are removed with a warning instead of being silently dropped
  during graph construction.
- `env`, `gen` and `proto` are indexable again, and an explicit `--include` now
  overrides the built-in excludes.
- `README.md` documents each adapter's known limits rather than listing
  aspirational features.

### Fixed

- **Property access resolved against a hardcoded variable name.** The new
  adapters compared the receiver against the literal string `"data"` — the name
  used in every fixture — so property tracking returned nothing for any other
  name in Rust, Go and C/C++. Receivers are now bound to declared types from
  parameters, `let`/`var` bindings, annotations and declarators, and properties
  are keyed per resolved type. One struct no longer inherits another's fields.
- **`#include` produced a graph with zero edges.** Includes became
  `local_header::<name>`, which `add_relationship` drops. They now resolve to
  the header's module symbol.
- **Every C struct name reported as ambiguous.** A `struct Foo *p` parameter is
  also a `struct_specifier`; treating it as a definition minted one symbol per
  referencing file. Structs and classes are recorded only where defined.
- Go imports pointed at an arbitrary member instead of the package; they now
  target a synthetic package symbol, and qualified references carry the
  specific type.
- Python indexed only some symbol kinds, so importing a function fabricated an
  `ext::<module>::package` dependency that did not exist.
- Rust use-paths matched leaf names repository-wide instead of resolving
  through the module that declares the name and following `pub use` chains.
- Go structural interface detection read method sets from the symbol table
  instead of splitting signature text on punctuation.
- An alias equal to the symbol's own name is no longer reported as a rename,
  which had buried genuine renames among ordinary references.
- The crates.io publish job now publishes all ten crates in dependency order,
  and the five new crates carry the metadata the registry requires. The release
  would previously have failed partway through, after `graphyn-core` was
  already published.

### Known limits

- Imports resolve within one language. A Python module importing a TypeScript
  file through a build step is not linked.
- Chained access is attributed to the first receiver only. In `a.b.c`, `c` is
  not attributed to the type of `a.b`.
- C++ templates are parsed but not instantiated.
- Go structural matching is per-package.
- Rust macro bodies are token trees; macro-generated code is not expanded.

## [0.1.3] - 2026-05-09

### Added

- Dependency-injection tracking and parallel parsing in `graphyn-core`.

### Fixed

- Windows path handling.

### Changed

- CI drops Intel macOS binaries; installers enforce Apple Silicon.

## [0.1.2] - 2026-04-28

### Added

- ESM and framework file support (`.vue`, `.svelte`, `.astro` script blocks).
- Durable re-export persistence.

### Changed

- Installer UX improvements and simplified `tsconfig` path sorting.

## [0.1.1] - 2026-04-18

### Added

- Cross-platform release pipeline and one-line installers for macOS, Linux and
  Windows.
- Crate publishing workflow.

## [0.1.0] - 2026-04-12

Initial release.

### Added

- `graphyn-core` graph engine, `graphyn-adapter-ts` TypeScript/JavaScript
  adapter, `graphyn-store` RocksDB persistence, `graphyn-mcp` MCP server and
  the `graphyn` CLI.
- `analyze`, `query blast-radius`, `query usages`, `query deps`, `status`,
  `watch` and `serve --stdio`.
- Configurable scan filtering and MCP `refresh_graph`.

[0.2.0]: https://github.com/JeelGajera/graphyn/compare/v0.1.3...v0.2.0
[0.1.3]: https://github.com/JeelGajera/graphyn/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/JeelGajera/graphyn/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/JeelGajera/graphyn/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/JeelGajera/graphyn/releases/tag/v0.1.0
