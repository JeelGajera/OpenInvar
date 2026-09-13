# OpenInvar MCP Configs

OpenInvar exposes MCP tools over stdio:

```bash
openinvar serve --stdio
```

Use these snippets as starting points. Set `OPENINVAR_ROOT` to the repository you
want OpenInvar to index/query.

## Cursor

Project file: `.cursor/mcp.json`

```json
{
  "mcpServers": {
    "openinvar": {
      "command": "openinvar",
      "args": ["serve", "--stdio"],
      "env": {
        "OPENINVAR_ROOT": "${workspaceFolder}"
      }
    }
  }
}
```

## Claude Code

Project file: `.mcp.json`

```json
{
  "mcpServers": {
    "openinvar": {
      "type": "stdio",
      "command": "openinvar",
      "args": ["serve", "--stdio"],
      "env": {
        "OPENINVAR_ROOT": "."
      }
    }
  }
}
```

Equivalent CLI:

```bash
claude mcp add-json openinvar '{"type":"stdio","command":"openinvar","args":["serve","--stdio"],"env":{"OPENINVAR_ROOT":"."}}' --scope project
```

### Antigravity

Antigravity reads MCP servers from `.gemini/settings.json`:

```json
{
  "mcpServers": {
    "openinvar": {
      "command": "openinvar",
      "args": ["serve", "--stdio"],
      "cwd": "${workspaceFolder}"
    }
  }
}
```

## Gemini CLI

Project or user `settings.json` entry:

```json
{
  "mcpServers": {
    "openinvar": {
      "command": "openinvar",
      "args": ["serve", "--stdio"],
      "env": {
        "OPENINVAR_ROOT": "."
      }
    }
  }
}
```

## OpenAI Codex CLI

User file: `~/.codex/config.toml`

```toml
[mcp_servers.openinvar]
command = "openinvar"
args = ["serve", "--stdio"]

[mcp_servers.openinvar.env]
OPENINVAR_ROOT = "."
```

## Tools

| Tool | Answers | Key arguments |
|---|---|---|
| `get_blast_radius` | What breaks if I change this symbol? | `symbol`, `file`, `depth`, `kinds`, `min_resolution` |
| `get_dependencies` | What does this symbol depend on? | `symbol`, `file`, `depth`, `kinds`, `min_resolution` |
| `get_symbol_usages` | Where is this used, including aliases? | `symbol`, `file`, `kinds`, `min_resolution` |
| `graph_diff` | What did this change break? | `base`, `head` |
| `check_rules` | Does this change violate `.openinvar/rules.toml`? | `base`, `head` |
| `refresh_graph_index` | Re-analyze after changes | `path`, `include`, `exclude`, `respect_gitignore` |

Six is a deliberate ceiling. A large tool surface degrades an agent's ability
to select the right tool, so a question that an existing tool already answers
does not get its own.

`graph_diff` and `check_rules` read snapshots and never analyze, so record one
first:

```bash
openinvar analyze . --snapshot HEAD
openinvar analyze . --snapshot worktree
```

Both default to comparing `HEAD` against `worktree`. Without a snapshot they
say so and name the command that records one, rather than analyzing on the fly
— an answer that depended on whatever was on disk at the moment of the call
would not be reproducible.

`check_rules` distinguishes four outcomes, and only one of them is a pass: a
rule can be satisfied, violated, undecided because the evidence in scope was
too weakly resolved to judge, or skipped because it needs a change that was not
supplied. A repository with no rules file is told that nothing was enforced
rather than that nothing was wrong.

## Common Checks

```bash
openinvar analyze .
openinvar status
openinvar serve --stdio
```

If a client cannot find `openinvar`, use the full path from:

```bash
which openinvar
```

