# Contributing Guide

[简体中文](CONTRIBUTING.md) | **English**

Welcome! This file is the **process guide** (issues, branches, commits, testing, PRs). The **coding standards** (style, comments, interfaces, gates) live in [AGENTS.md](AGENTS.md) and apply equally to human contributors and AI coding agents. The Chinese version is canonical for discussion and commit subjects.

- Channels: [GitHub Discussions](https://github.com/Atom112/AIO/discussions) (Q&A, ideas), QQ group **1130943633** (Chinese community)
- Security reports: never open a public issue — see [SECURITY.md](SECURITY.md)

---

## 1. Filing Issues

Search existing [Issues](https://github.com/Atom112/AIO/issues) and [Discussions](https://github.com/Atom112/AIO/discussions) first.

| Type           | Entry                                                                                        | Requirements                                                                                                                                                                                                           |
| -------------- | -------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Bug            | [Bug report form](https://github.com/Atom112/AIO/issues/new?template=bug_report.yml)         | Attach the environment snapshot (Settings → App → Feedback & Support → copy) and minimal reproduction steps                                                                                                            |
| Feature idea   | Start in [Discussions "Ideas"](https://github.com/Atom112/AIO/discussions)                   | After enough 👍 the maintainer opens a tracking issue; if you file directly, use the [feature form](https://github.com/Atom112/AIO/issues/new?template=feature_request.yml) — describe the problem before the solution |
| Alpha feedback | [Alpha feedback form](https://github.com/Atom112/AIO/issues/new?template=alpha_feedback.yml) | One-line verdict + satisfaction + blockers/delighters                                                                                                                                                                  |

Labels (maintained by maintainers): `bug` / `enhancement` / `needs-triage` / `area:*`. One issue per problem.

## 2. Branching Model

Two long-lived branches: **`dev` (integration)** and **`master` (released content only)**. All changes enter `dev` via PR.

- **Feature branches**: cut from the latest `dev`, named `dev-<type>-<topic>` (e.g. `dev-feat-memory-panel`, `dev-fix-settings-jank`). Do not use `dev/<name>` — Git cannot create that namespace while the `dev` branch exists.
- **One branch, one topic**: a feature or a fix, independently reviewable and revertible.
- **Stay fresh**: merge `dev` into your branch regularly; do not rebase pushed branches.
- **Type prefix** in the name (`feat` / `fix` / `docs` / `chore` / `refactor`), mirroring the commit type.
- **External contributors**: fork, branch in your fork, PR back to `Atom112/AIO:dev`.
- **Releases** (maintainer): `dev` → `master` PR → bump versions + CHANGELOG → push tag `vX.Y.Z` to trigger [release.yml](.github/workflows/release.yml). Prerelease tags (e.g. `v1.0.0-alpha`, hyphenated) are automatically marked as GitHub prereleases, so `releases/latest` always points at the stable line.

## 3. Commit Messages

Follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) — `type(scope): subject`. The type must come from this table (mapped 1:1 to [cliff.toml](cliff.toml)):

| type          | for                            |
| ------------- | ------------------------------ |
| `feat`        | new features                   |
| `fix`         | bug fixes                      |
| `docs`        | documentation                  |
| `perf`        | performance                    |
| `refactor`    | refactors (no behavior change) |
| `style`       | formatting (no logic change)   |
| `test`        | tests                          |
| `chore(deps)` | dependency upgrades            |
| `chore`       | build / tooling / misc         |
| `revert`      | reverts                        |

- **Subject**: Chinese, one line stating what changed, no trailing period.
- **Body**: motivation and approach (the _why_), not a diff narration.
- **References**: `Closes #123`; breaking changes carry a `BREAKING CHANGE:` footer with migration notes.
- **Honest attribution**: AI-agent changes follow every rule here; no double standard.

## 4. Local Testing

See the [development environment guide](docs/en/development/getting-started.md). The pre-commit baseline (identical to CI):

```bash
npm run verify
```

- `npm run check:frontend`: i18n key parity → doc links → architecture/style gates (`check:gates`) → prettier → eslint → tsc → vitest → vite build
- `npm run check:rust`: cargo fmt → clippy (`-D warnings`) → cargo test

Cadence: each commit at least runs relevant tests (`npx vitest run <pattern>` / `cargo test <name>`); run full `npm run verify` before opening a PR.

**Gates are hard**: `check:gates` enforces rules (no bare-Chinese JSX, no hardcoded colors, `any` ratchet, component layering, file-size caps, hidden-console for child processes). Never loosen a gate to make a commit pass — gate changes are their own PR with justification. `--no-verify` is forbidden.

**Test authoring**: colocate frontend tests with sources (`*.test.ts`); prefer inline `#[cfg(test)]` modules in Rust, heavy integration tests in `src-tauri/tests/`. Bug-fix PRs include a regression test (red first, then green).

## 5. Pull Requests

1. Branch from the latest `dev`, develop, self-verify (§4).
2. Open a PR against `dev` following the [template](.github/PULL_REQUEST_TEMPLATE.md): what, why, verification evidence (commands + results), UI notes.
3. CI ([ci.yml](.github/workflows/ci.yml)) must be green before review.
4. A [CODEOWNERS](.github/CODEOWNERS) maintainer approves, then merge; delete the feature branch afterwards.
5. Plain merge commits; never force-push `dev` / `master`.

Keep PRs small and focused. Split big features into independently mergeable PRs (see the "PR A/B/C/D/E" precedents in the CHANGELOG).

## 6. AI-Agent-Assisted Development

AIO is developed heavily with AI coding agents, and contributors are welcome to do the same. Agent conduct follows the "Agent Collaboration Rules" section of [AGENTS.md](AGENTS.md). Core red lines:

- **Verification honesty**: never claim unexecuted tests pass; claims come with commands and results.
- **No gate bypass**: no `--no-verify`, no threshold loosening, no test-special-casing.
- **Read before asserting**: source code beats speculation.
- **Minimal diffs**: no drive-by refactors; adjacent problems get an issue, not a drive-by fix.
- **PRs mandatory**: agent output goes through the same PR flow; agents never push to `dev` / `master`.

## 7. Documentation and Screenshots

- **Bilingual mirror**: `docs/zh/` and `docs/en/` are file-by-file mirrors; substantive edits must update both sides in the same PR. Links must pass `npm run check:docs`.
- **Docs follow code**: behavior changes ship with doc updates; unshipped plugins are explicitly marked (see the note at the top of [docs/en/README.md](docs/en/README.md)).
- **Screenshot discipline**: no personal data, real conversations, or machine paths. Use a clean data profile or fabricated demo content. 1600x1000 webp under `docs/assets/screenshots/`.
- **No one-off artifacts**: interim reports, review logs, and phase plans do not live in `docs/` — put them in PR descriptions or issues. `docs/` keeps only documentation that evolves with the product.
- **CHANGELOG**: changes accumulate under `## [Unreleased]` during development; the maintainer curates release entries.

## 8. Security and Privacy Red Lines

- Never commit API keys, tokens, personal paths, or real user data; credentials go through the OS keychain (`secure_store`).
- Never expose keys or conversation content in issues, screenshots, or diagnostics (the in-app diagnostic zip is sanitized).
- Report vulnerabilities privately per [SECURITY.md](SECURITY.md).

---

Thank you! AIO is a personally maintained project — review and merging may take a few days.
