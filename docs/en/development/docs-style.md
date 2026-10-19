# Documentation Style Guide

This guide governs writing and maintaining everything under `docs/` plus the READMEs. The audience is **contributors** (humans and AI agents alike). The overarching principle: documentation follows the code and the release workflow — short and accurate beats long and stale.

## Document System and Boundaries

| File                            | Audience                   | Scope                                                                                          |
| ------------------------------- | -------------------------- | ---------------------------------------------------------------------------------------------- |
| `README.md` / `README.zh-CN.md` | All visitors               | Project pitch, feature overview, quick start, doc navigation, community; no dev internals      |
| `CONTRIBUTING.md` (bilingual)   | Contributors               | Process rules: issues / branches / commits / PRs / releases                                    |
| `AGENTS.md`                     | Contributors and AI agents | Single source of truth for coding standards: style, comments, interfaces, gates                |
| `docs/zh                        | en/usage/`                 | End users                                                                                      | Feature-oriented user guides (getting started, chat & agent, harness, MCP, ...) |
| `docs/zh                        | en/development/`           | Developers                                                                                     | Dev environment, architecture, extension guides, this style guide               |
| `docs/zh                        | en/reference/`             | In between                                                                                     | Lookup material: configuration reference, permission model                      |
| `CHANGELOG.md`                  | All users                  | User-facing version history (Keep a Changelog spirit: per version, human-readable, with links) |

## Bilingual Mirror Rules

- `docs/zh/` and `docs/en/` mirror **file by file**: adding, renaming, or deleting a document happens on both sides in the same PR.
- Substantive content changes must be mirrored; even typo fixes are best done on both sides.
- The English side is not machine translation: terminology and tone follow English technical writing conventions (see existing `docs/en/` prose), but **facts, structure, and link targets must match the Chinese side**.
- The bilingual READMEs and CONTRIBUTINGs follow the same pairing rules.
- Verification: `npm run check:docs` walks all Markdown links (including anchors) on both sides — it is the floor for sync compliance, not the ceiling; structural changes still need human cross-checking.

## Writing Rules

- **Structure**: each document starts with `# Title`; H2 for themes, H3 for subtopics; never skip heading levels. Nesting beyond three levels usually means the document should be split.
- **Paragraphs and lists**: ordered lists for procedures; unordered lists for enumerations; keep list items structurally consistent (all short phrases or all full sentences). Tables are for multi-dimensional comparisons (mode tables, config entries, support matrices).
- **Code blocks**: annotate the language (`bash / `rust / ```tsx); include only copy-runnable commands and minimal working examples, never fake code; inline-code backticks around identifiers, commands, and paths.
- **Links**: relative links for in-site docs (`[Chat and Agent](../usage/chat-and-agent.md)`); anchors for sections (anchor spelling must pass `check:docs`); canonical URLs for external references.
- **Wording**:
  - Describe "what it is / how to use it"; no marketing language ("powerful", "perfect");
  - Avoid unfulfilled promises ("coming soon", "will eventually") — either omit undelivered capability or mark its status explicitly;
  - Version-sensitive statements carry the version ("since v0.8").
- **Screenshots**: only for UI explanation; **never include** personal data, real conversation content, machine-absolute paths, or API keys. Use a clean profile or fabricated demo content; 1600x1000 webp under `docs/assets/screenshots/`, linked relatively.
- **Voice**: address users as "you"; developer docs use first-person plural or impersonal sentences.

## Maintenance Discipline

- **Docs follow code**: a behavior-change PR ships its doc updates (the PR template has a checkbox); code without docs is not done.
- **Delete over annotate**: when a feature is removed, delete its doc sections — no "historical remains" chapters; history belongs to CHANGELOG and git.
- **No one-off artifacts**: interim reports, review logs, phase plans, and environment-issue write-ups do not go into `docs/`; they belong in PR descriptions, issues, or local notes. `docs/` keeps evergreen documentation that evolves with the product.
- **Data and examples**: all example data in docs is fabricated (`demo-project`, `example.com`, `your-api-key`); never copy from real environments.
- **Verification checklist**: run `npm run check:docs` after doc changes.

## Relation to Other Standards

- In-code comment and interface documentation rules live in [AGENTS.md](../../../AGENTS.md) ("Code Comments" and "Interface Contract").
- Commit and PR processes (including doc PRs) live in [CONTRIBUTING.md](../../../CONTRIBUTING.md).
- Revisions to this guide follow the same PR + bilingual-sync rules.
