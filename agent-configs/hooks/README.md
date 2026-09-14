# OpenInvar Hooks

MCP is pull-only: the agent has to decide to ask. Hooks are push — the graph
reaches the agent at the moment of the edit, whether or not it thought to look.

| Hook | Fires | Does | Can block? |
|---|---|---|---|
| `claude/openinvar-pre-edit.sh` | `PreToolUse` on Edit/Write | Injects the target file's blast radius into context | No |
| `claude/openinvar-post-edit.sh` | `PostToolUse` on Edit/Write | Reports references the edit broke | No |
| `claude/openinvar-stop-check.sh` | `Stop` | Runs `openinvar check --diff-only`; feeds the failing rule back | Yes |
| `git/pre-commit` | `git commit` | Runs `openinvar check --diff-only` | Yes |
| `git/post-commit` | after `git commit` | Records the new `HEAD` snapshot | No |

## Install — Claude Code

```bash
mkdir -p .claude/hooks
cp agent-configs/hooks/claude/*.sh .claude/hooks/
cp agent-configs/hooks/lib/openinvar-hook-lib.sh .claude/hooks/
chmod +x .claude/hooks/*.sh
```

Then merge `agent-configs/hooks/claude/settings.json` into `.claude/settings.json`.

## Install — git

```bash
cp agent-configs/hooks/git/pre-commit  .git/hooks/pre-commit
cp agent-configs/hooks/git/post-commit .git/hooks/post-commit
cp agent-configs/hooks/lib/openinvar-hook-lib.sh .git/hooks/
chmod +x .git/hooks/pre-commit .git/hooks/post-commit

openinvar analyze . --snapshot HEAD   # once, to create the first baseline
```

`post-commit` is not optional if you want `pre-commit` to keep working.
`check --diff-only` compares `HEAD` against the working tree, and `HEAD` moves
with every commit — without `post-commit` the baseline goes stale immediately
and the gate reports "rules were not enforced" forever. That message is honest,
but a gate nobody has enabled correctly is a gate that catches nothing.

## Worked example

A rule saying the payload type may not lose fields:

```toml
# openinvar.toml
[[rule]]
name = "payload-is-stable"
kind = "no-field-removal"
symbol = "UserPayload"
```

An agent removes `email` from that class and tries to commit:

```
$ git commit -m "drop unused email field"

  ─── Violations ─────────────────────────

    payload-is-stable no-field-removal [error]
        src/models/user_payload.ts:5
          field 'email' removed from 'UserPayload'

  ✗ 1 rule(s) violated.

openinvar: commit blocked by a rule in openinvar.toml
         Override once with: git commit --no-verify
```

Before the edit, the `PreToolUse` hook had already put this in the agent's
context:

```
OpenInvar blast radius for src/models/user_payload.ts: 1 file(s) and 3
reference(s) depend on symbols defined here. Check `openinvar impact` before
renaming or removing anything in this file.
```

## Design rules these scripts follow

**Never hang.** Every call is wrapped in a timeout — `timeout(1)` where it
exists, a background process and a poll where it does not, because stock macOS
has no `timeout`. A hook that makes an agent wait is a hook that gets switched
off, and a switched-off hook enforces nothing.

**Never block on a tool problem.** Only one condition blocks: a rule violated
on resolved evidence. A missing binary, a missing graph, a stale snapshot, a
timeout, an unreadable rules file — all exit 0. `openinvar check` distinguishes
these itself: exit 1 is a broken rule, exit 2 is a check that could not run,
and the hooks act only on exit 1. Trapping an agent, or rejecting a colleague's
commit, because a tool was misconfigured is worse than not running.

**Never claim a check that did not happen.** Where a hook cannot enforce, it
says so on stderr rather than exiting quietly. Silence and success look
identical, and a gate that silently stops working is the failure mode that
matters.

**Say nothing when there is nothing to say.** The pre-edit hook emits no
context for a file with no dependents. An agent's context window is not free.

## Configuration

| Variable | Effect |
|---|---|
| `OPENINVAR_BIN` | Path to the binary, if not on `PATH` |
| `OPENINVAR_HOOK_TIMEOUT` | Per-call timeout in seconds |
| `OPENINVAR_SKIP` | Set to any value to disable the git hooks for one command |

`git commit --no-verify` skips the git hooks the usual way.

## Requirements

POSIX `sh`. `jq` is used when present and a smaller fallback parser is used
when it is not; both are exercised by the test suite.

## Other agents

The git hooks are agent-agnostic and are the recommended integration for any
tool that is not Claude Code — they run at the commit boundary regardless of
what produced the edit.

Codex and other agents are covered by the instruction files in the parent
directory (`AGENTS.md`, `cursor/`, `gemini/`) plus the git hooks. OpenInvar does
not ship a per-edit hook for them: their hook APIs were not verified against
primary documentation when these scripts were written, and shipping a template
built on a guessed API would break in someone's repository rather than ours.
