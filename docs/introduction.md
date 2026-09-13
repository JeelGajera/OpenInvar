# Introduction

> Deterministic integrity checks for AI-written code — structural violations, test tampering, contract erosion. No model in the loop.

OpenInvar reads a change as a graph of symbol relationships and reports what the change actually did: a test that stopped covering a symbol the same diff edited, a symbol removed while a caller still refers to it, an architectural constraint the repository wrote down and this change broke.

It is not a search tool. It is not a chatbot over your repo. No model participates in building the graph or in any gating decision, so identical input produces identical output, byte for byte — which is the only reason a gate can act on the result.

## Core Features

- **`audit`**: Detect changes made to pass a check rather than to work — test tampering, contract erosion, code only its own tests reach.
- **`check`**: Enforce the architectural rules a repository wrote down, across every language in it, exiting non-zero when one breaks.
- **Alias resolution**: Tracks imports like `import { A as B }` across your entire project, so a rename finds the callers a text search misses.
- **Property-level tracking**: Understands not just that a class is used, but which specific properties are accessed.
- **MCP Integration**: Speaks standard Model Context Protocol, making it compatible with Cursor, Claude Code, and other modern agents.
- **Honest coverage**: Reports per-language resolution on every run, and gates fail open on regions it could not resolve.

## Getting Started

To install OpenInvar, follow the instructions in the [README](https://github.com/JeelGajera/OpenInvar#install). Once installed, you can start analyzing your repository:

```bash
openinvar analyze ./my-repo
```
