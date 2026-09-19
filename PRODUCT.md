# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

<!-- Deviation, stated: the platform is a terminal UI (Rust, ratatui + crossterm), not web. The schema has no `tui` value; `web` is recorded only to keep the record parseable. Every web-specific directive (viewports, CSS, browser screenshots) maps to: terminal sizes 80×24 and 200×60, monochrome + 16-color + truecolor terminals, and captured terminal frames. -->

## Stack

Rust workspace. CLI and MCP exist (`crates/cli`, `crates/mcp`, core in `crates/core`). Human UI: **ratatui + crossterm**, same binary (`swamp` with no subcommand, or `swamp ui`). Confirmed by the maintainer 2026-09-18.

## Users

A developer who runs many coding agents (Claude Code, Codex, …) on one Mac, with dozens of git checkouts and linked worktrees under `~/src`, Docker images/build cache, and recurring, sudden disk growth they did not personally cause. Two operators of the same data: an AI agent (via MCP/CLI — the primary operator) and the developer at a terminal (this surface), typically when free space just got tight or when a weekly look is due.

## Product Purpose

Explain sudden storage growth per git project → checkout/worktree → artifact type, since the previous observation, with unowned bytes reported honestly; then let the human act on artifacts in one keystroke with confirmation. Success: the developer sees what grew and why in one screen, acts on rebuildable artifacts without leaving the tool, and stops doing emergency `du` sessions.

## Positioning

The only tool that combines a project/worktree model of the filesystem with persistent growth history (reverse-delta column store), Docker attribution by explicit evidence, and an agent-operable interface. kondo/npkill are stateless and path-shaped; StorageRadar has history but no project model and a GUI only.

## Operating Context

macOS terminal (iTerm2/Terminal.app/Ghostty/tmux), often inside or beside a coding-agent session. Trees of 40–400 GB, 50–100 checkouts, thousands of directories. Observations are taken by the CLI, by a scheduled job (planned, #31), or on open. Deletions go to Trash.

## Capabilities and Constraints

- Data comes from `swamp_core::report` — the same Report the CLI/MCP render. The TUI adds no second data path.
- Views: overview (projects), project tree (checkouts → worktrees → artifacts → dirs), kinds, docker, unowned. Filter language: `growth > <size> in last <duration>`; default on open `growth > 100 MB in last 7 days` (confirmed).
- Actions: **Backspace** marks the selected unit for deletion; **Enter** confirms; deletable units are **folded artifacts only** (dependency trees, build outputs, caches, Docker objects) — never a checkout, worktree, `.git`, Source tree, or unowned filesystem path (confirmed). Deletion routes through the parked action layer (`grants`/`execution`/`ledger`): Trash by default, live re-derivation at the sink, per-unit outcome record, occupancy protection. A human pressing Enter at the keyboard is the authorization for that one action.
- Hard constraints inherited from the epic (#19): index is evidence, never authorization; no verdict vocabulary ("safe", "stale", "unused", "abandoned"); worktree staleness is a signal, never a verdict; folded trees are one unit; totals reconcile to the walk; nothing read from disk is an instruction.
- Terminal constraints: must work at 80×24 and in 16-color terminals; no mouse required; keys documented in a one-line footer.

## Brand Commitments

Name: swamp. Voice: dense, factual, dry; numbers first, no exclamation marks, no emoji. No logo.

## Evidence on Hand

Real data on the maintainer's machine (`swamp report ~/src`): 63 projects, 103 worktrees, ~41 GB attributed, Docker 15.8 GB unowned. Fixture with golden expectations in `crates/core/tests/fixture/`. No testimonials, no benchmarks beyond those in issue comments.

## Product Principles

1. Growth first: the default view answers "what grew" before "what is big."
2. One unit, one key: every actionable thing is a folded unit with a stable identity; acting on it is one keystroke plus one confirmation.
3. Facts, not verdicts: the UI shows signals and lets the human decide; it never says "safe".
4. Same truth everywhere: TUI, CLI, and MCP render one Report; nothing is computed only for the screen.
5. Honest coverage: unowned, permission-denied, and Docker-outside-the-walk are always visible, never hidden to make a number look complete.
