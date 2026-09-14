# Security Policy

## Reporting a vulnerability

**Please do not report security vulnerabilities through public issues, pull
requests or discussions.**

Report privately through GitHub:

1. Go to the [Security tab](https://github.com/JeelGajera/OpenInvar/security)
2. Choose **Report a vulnerability**

This opens a private advisory visible only to the maintainers. GitHub's
[private vulnerability reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability)
documents the flow.

A useful report includes the version (`openinvar --version`), what an attacker
would gain, and the smallest input that demonstrates it. If the finding
involves a repository or file that triggers the behaviour, attach it rather
than describing it — reproduction is most of the work.

You can expect an acknowledgement within a week. If a report is confirmed, the
fix and a GitHub Security Advisory are published together, and you will be
credited unless you ask not to be.

## Supported versions

OpenInvar is pre-1.0 and moves quickly. **Only the latest release receives
security fixes.** There are no maintained release branches; a fix ships in the
next version rather than being backported.

| Version | Supported |
|---|---|
| Latest release | ✅ |
| Anything earlier | ❌ |

## What is in scope

OpenInvar's security surface is **reading code it did not write**. It runs over
whatever repository it is pointed at, frequently in CI, frequently on branches
that came from outside the project. Treat analysed source as untrusted input,
because it is.

In scope:

- **Parser and analyzer crashes or memory-safety failures** on crafted source
  files. A panic that stops a CI job is a denial of service worth reporting.
- **Path traversal on analysis or output** — a repository whose contents cause
  reads or writes outside the analysed tree.
- **Anything that makes OpenInvar execute analysed code.** It parses; it must
  never evaluate, import, link or run what it reads. A report that breaks this
  is the most serious class here.
- **Store corruption or escape** — crafted input that lets a repository control
  the SQLite store beyond the graph it is supposed to describe.
- **A gate reporting a pass it did not earn**, where the input was crafted to
  produce that result. OpenInvar exists to block changes; making it fail open
  on purpose defeats its stated purpose. (An *honest* `Inconclusive` is the
  designed behaviour, not a vulnerability — see
  [CONTRIBUTING.md](CONTRIBUTING.md#the-four-verdicts).)
- **The installers** ([`install.sh`](install.sh), [`install.ps1`](install.ps1))
  and the [GitHub Action](action.yml), including how they resolve and verify
  what they download.
- **The MCP server** (`openinvar serve --stdio`), which speaks to agent
  runtimes on behalf of a user.

## What is out of scope

- **Findings in code OpenInvar analyses.** OpenInvar is not a vulnerability
  scanner and does not claim to be; a security flaw it fails to notice in your
  repository is not a flaw in OpenInvar.
- **Missing, wrong or incomplete edges** in ordinary use. Resolution gaps are
  bugs — please
  [file them](https://github.com/JeelGajera/OpenInvar/issues) — but they are
  not vulnerabilities, and the
  [Known limits](README.md#known-limits) section documents the ones already
  understood.
- **Vulnerabilities in tree-sitter grammars or other dependencies**, which
  belong upstream. Tell us anyway if OpenInvar's use makes one reachable that
  otherwise would not be, and we will coordinate.
- **Resource use on very large repositories** in the absence of a crafted
  trigger.

## What OpenInvar does not do

Worth stating plainly, because it bounds the surface:

- **It never executes the code it analyses.** Everything is derived from
  tree-sitter parse trees. There is no evaluation, no import, no build step and
  no plugin mechanism in the analysis path.
- **No LLM or network service participates in analysis.** Analysis is local and
  offline; your source is not sent anywhere. This is a core product property,
  not only a security one.
- **It writes only inside the analysed repository**, under `.openinvar/`.
