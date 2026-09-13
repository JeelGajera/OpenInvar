# OpenInvar Agent Setup Pack

Copy these templates into a project that uses OpenInvar so coding agents know when
and how to query the graph before changing code.

## What To Use

| Agent / IDE | Template in this folder | Usual project location |
|---|---|---|
| OpenAI Codex / AGENTS.md-aware tools | `AGENTS.md` | `AGENTS.md` |
| Claude Code | `claude/CLAUDE.md` | `CLAUDE.md` or `.claude/CLAUDE.md` |
| Claude Code Skill | `claude/skills/openinvar/SKILL.md` | `.claude/skills/openinvar/SKILL.md` |
| Cursor | `cursor/rules/openinvar.mdc` | `.cursor/rules/openinvar.mdc` |
| GitHub Copilot | `github/copilot-instructions.md` | `.github/copilot-instructions.md` |
| Gemini CLI / Gemini-style agents | `gemini/GEMINI.md` | `GEMINI.md` |
| Antigravity-style rules | `antigravity/rules/openinvar.md` | `.agents/rules/openinvar.md` |
| Antigravity-style workflows | `antigravity/workflows/openinvar-impact-check.md` | `.agents/workflows/openinvar-impact-check.md` |
| MCP clients | `mcp/README.md` | Client-specific config |
| Hooks (Claude Code + git) | `hooks/` | `.claude/hooks/`, `.git/hooks/` |

## Recommended Setup

1. Install OpenInvar and run `openinvar analyze .` once.
2. Add the right instruction file for your agent.
3. Add the MCP config from `mcp/README.md` if your agent supports MCP.
4. Tell agents to use OpenInvar before risky edits: renames, deleting symbols,
   changing public types, DTOs, services, mappers, or shared utilities.
5. Install the hooks from `hooks/` if you want this to happen without the agent
   deciding to ask. Instruction files are advice; hooks are not.

## Notes

- `AGENTS.md` is the best shared baseline because more coding agents understand
  it over time.
- Platform-specific files are still useful because some tools read only their
  own instruction path or support richer behavior there.
- Keep these files short. Agents follow focused operational rules better than
  long product documentation.

## Filtering For Agents

OpenInvar respects `.gitignore` by default. If an agent cannot find a symbol, first
check whether the target file is ignored or outside the current include filters.

CLI overrides:

```bash
openinvar analyze . --no-gitignore
openinvar analyze . --include "src/**/*.ts"
openinvar analyze . --exclude "tests/**"
```

MCP refresh options:

```json
{
  "path": ".",
  "respect_gitignore": false,
  "include": "src/**/*.ts",
  "exclude": "tests/**"
}
```

## Format References

- AGENTS.md: https://github.com/openai/agents.md
- Claude Code memory: https://docs.claude.com/en/docs/claude-code/memory
- Claude Code Skills: https://docs.claude.com/en/docs/claude-code/skills
- Claude Code MCP: https://docs.claude.com/en/docs/claude-code/mcp
- Cursor rules/MCP: https://docs.cursor.com
- GitHub Copilot custom instructions: https://docs.github.com/en/copilot/how-tos/custom-instructions/adding-repository-custom-instructions-for-github-copilot
- Gemini context: https://google-gemini.github.io/gemini-cli/docs/cli/gemini-md.html
- Gemini MCP: https://google-gemini.github.io/gemini-cli/docs/tools/mcp-server.html
