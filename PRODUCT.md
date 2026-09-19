# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

<!-- Schema compatibility only: the product is a macOS terminal application. The schema has no terminal value. Use terminal frames at 80×24 and 200×60 for visual review. -->

## Stack

Rust workspace. `swamp-core` builds the report and provides storage and action primitives. `swamp` provides the CLI and a ratatui/crossterm UI; `swamp-mcp` provides a stdio MCP server.

## Users

Developers with multiple checkouts, linked worktrees, build systems, and Docker workloads on one Mac. Coding agents add another source of builds and dependency installs. Developers and their agents need to locate storage growth and inspect the affected project before acting.

## Product Purpose

Explain disk growth by Git project, checkout or worktree, and artifact. Keep observations so users can compare changes over time, inspect the relevant context, and remove selected units from the same tool.

## Positioning

Swamp combines a development-specific storage model with observation history, incremental filesystem updates, Git and Docker context, and CLI, TUI, and MCP interfaces. Describe these capabilities directly; do not claim exclusive ownership of disk-usage history or use an unverified competitor matrix.

## Operating Context

macOS terminal. Observations come from report commands, MCP report tools, the open TUI, or an optional LaunchAgent. The TUI watches for filesystem events while open. Scheduled observation refreshes data without performing cleanup.

## Capabilities and Constraints

- Core report data is shared across interfaces. Filtering, display, and action orchestration differ; see the [usage reference](docs/usage.md).
- The initial TUI filter is `growth > 100MB in 7d`; saved choices override it. Views cover projects, trees, builds, dependencies, Docker, kinds, unowned storage, and ecosystems.
- History starts with observation and is bounded by retained measurements. It contains metadata and sizes, not file contents or writer identity. See [known implementation limits](docs/architecture.md#limits-of-the-current-implementation).
- Artifacts, source directories, whole checkouts, linked worktrees, and some unowned paths can be selected for actions. Confirmation must make the selected scope and recovery behavior visible.
- Filesystem paths go to Trash. Docker images and volumes are removed by the daemon without a backup. Build-cache records are report-only.
- Git status and activity are evidence for the user's decision. Dirty, unpushed, and untracked warnings do not universally block removal.
- Plans require authorization. MCP has no grant-writing tool; this does not prevent a process with shell access from invoking the CLI.
- Coverage and unowned storage remain visible. Docker has separate reconciliation totals from the filesystem walk.

## Brand Commitments

Name: swamp. Use concrete descriptions, documented commands, and qualified measurements. Ecosystem glyphs identify project types in the terminal; prose needs no decorative icons or slogans.

## Evidence on Hand

The implementation, committed report and TUI fixtures, and integration tests are the primary evidence. Historical timings in the changelog came from individual developer trees, not a controlled benchmark. The [documentation accuracy report](docs/accuracy.md) records checked claims and corrections.

## Product Principles

1. Lead with growth, with size available alongside it.
2. Preserve project and worktree context when inspecting individual artifacts.
3. Show the paths, warnings, and recovery behavior before an action.
4. Distinguish measured data, cached facts, missing evidence, and unknown ownership.
5. Keep repeated observations cheap enough to make history practical; verify this with representative workloads.
