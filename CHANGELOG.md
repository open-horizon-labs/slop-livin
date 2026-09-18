# Changelog

Release notes live here, one section per tag. The release workflow refuses a tag without one.

## v0.2.0

The clean-dev-dirs salvage: everything it does for a human, on top of the history and project model it doesn't have.

**Project types, everywhere**

- Twenty ecosystems detected from checkout-root markers (`Cargo.toml`, `package.json`, `pyproject.toml`, `go.mod`, `pom.xml`, `*.csproj`, …). Each project row wears its glyphs (🦀 ⬢ 🐍 🐹 ☕ 🐦 🟣 💎 …), plus 🐳 for Docker objects, 🔨 for build output, ⎇N for linked worktrees. The CLI overview gained a `type` column.
- **Which artifacts go where.** The same table says which directories each ecosystem generates. Ambiguous names (`build`, `dist`, `vendor`, `bin`, `obj`, `out`, `*.egg-info`) are artifacts only next to a marker of an ecosystem that generates them; a hand-written `build/` in a repo with no build tool is source. Every artifact row carries the ecosystem that produced it.
- `type:rust` filter (CLI, TUI, MCP, grants), `t` sort grouping by type, a **types view** (`8`, `--view types`, MCP `view: types`) with projects, artifacts, bytes and growth per ecosystem, and `summary.by_type` in the JSON report.
- A checkout without a remote is named from its manifest (`[package] name`, `"name"`, `module`, `<artifactId>`, `name:`, `app:`, `project(...)`, `*.csproj`).

**Filters, sorts, the form**

- `size > 500MB` / `size < 1GB` and `age > 30d` (time since an artifact was last written; newest mtime is now recorded per artifact and kept in the store). `project:` accepts globs (`my-app*`). Sizes parse decimal (`500MB`) and binary (`500MiB`), matching what the tool prints.
- Sort by name (`n`), type (`t`), age (`a`); `r` reverses; CLI `--sort growth|size|name|type|age --reverse`. Sort, reverse and the filter persist across sessions.
- The `/` form gained type, size and age fields; `:` completion knows `size >`, `age >` and every `type:` tag.

**Keep executables**

- `k` in the TUI, `--keep-executables` on `execute`, `keep_executables` on the MCP tool: before a build directory goes to Trash, Rust `target/{release,debug}` executables and Python `dist/*.whl` / `build/**/*.so` are copied to `<worktree>/bin/`. Outcomes list what was kept; the ledger records it.

**Honest progress, honest refresh**

- `observing…` in the TUI header and on the CLI's stderr shows real bytes and directories walked so far, with a percentage against the last observation. It used to say `0%` and mean nothing.
- Deleting in the TUI removes the row and everything under it at once, adjusts the header totals, and starts an incremental (FSEvents) re-observe in the background. Before, only exact-path artifact rows were dropped and nothing re-observed.
- A change to the classification rules forces one full walk so rows that no longer count leave the store instead of lingering.

**Charts**

- Sparklines are drawn by ratatui's `Sparkline` widget from honest data: buckets before a row's first observation are `·` (not zero), flat rows draw nothing, and the header shows the whole root's history with its net change.

**Architecture: the pipeline is on the bus**

- The report is built by twelve consumers on an in-memory event bus (tokio, static registration, dynamic routing) instead of one thousand-line function: walk → projects → {signals, ecosystems, docker} → github → gate → growth store → tracking → history → assemble → cache. Reports are byte-identical to before. `docs/ADRs/001-event-bus-report-pipeline.md`.
- Every technical constraint of the design (FSEvents before any walk, Parquet + zstd, reverse deltas, scheduled refresh, folding only for artifacts, symlinks never followed, incremental re-walks, the parallel pool, int32-minute mtimes, facts not verdicts, human-only authorization, pluggable consumers, one byte formatter) is a guardrail under `.oh/guardrails/` and a named AST audit in `crates/source-audit`; `scripts/check.sh` fails when one is broken.

**Also**

- `slop-livin config show | path | init`: the config file, every key with its meaning.
- `filter:` parameter on the MCP `report` tool, same grammar as everywhere else.

## v0.1.0

First release. `slop-livin` answers "what grew on this disk, by project, and what do I do about it" for developers running many coding agents at once — and lets you or your agent act on the answer with the facts in front of you.

**What you get**

- **Terminal UI** (`slop-livin ui ~/src`): projects sorted by growth, a tree per project (checkouts → worktrees → artifacts → directories), a history sparkline on every row, a filter form (`/`) and a filter line with Tab completion (`:`). Space marks, Backspace deletes what's under the cursor after one confirm that states the facts — `dirty · 26 unpushed · raw untracked 1.1GB` — and everything goes to Trash.
- **CLI** (`slop-livin report`): the same report as one screen, a project tree, and named views (`worktrees`, `builds`, `deps`, `docker`, `kinds`, `unowned`, `reconciliation`), with `--json`.
- **MCP server** (`slop-livin-mcp`): `report`, `what_grew`, `list_projects`, `list_worktrees`, `docker_objects`, `propose`, `execute`, `plans`, `grants`. An agent can propose and execute; only a human at the CLI can authorize (`slop-livin approve <plan>` or a bounded standing `grant`).
- **Growth over time.** Every observation goes into a Parquet column store with reverse deltas, so growth over any window is a lookup. Growth windows are capped at the history the store actually holds, and the header says so.
- **Incremental observation** via FSEvents: re-walk only what changed, byte-identical to a full walk. Full walk of ~41 GB / 107 worktrees: ~7.5 s; incremental: 2–3 s (one machine).
- **Scheduled observation**: `slop-livin schedule --every 15m ~/src` installs a low-priority LaunchAgent so history accumulates without you.
- **Projects, not paths.** Identity is the git object store, unified across clones by remote; rows read `owner/repo`. Artifacts (`node_modules`, `target`, `build`, `.venv`, `.cache`, …, ~50 names across 20 ecosystems) are one unit each and attributed to the nearest checkout.
- **Git status on every row**: `tracked`, `ignored`, `untracked`. Untracked bytes are in no version control and under no ignore rule — nothing brings them back.
- **Worktree facts**: last commit age, dirty, unpushed, locked, idle; with `gh`, PR state and whether the branch is merged, shown as `merge-complete` with all its terms. Never a verdict word.
- **Docker** images, build cache and volumes attributed to projects only on explicit evidence (compose labels, compose-file `name:`, `image.source` matching a remote); everything else listed as unowned with the reason.
- **Honest totals**: `attributed + unowned = walked` to the byte; `--verify-du` as an oracle.

**Known limits**

- macOS, Apple silicon only. Binaries are unsigned (see install note).
- History starts when you install; the first day's growth windows are short.
- Docker objects can be seen and attributed but not yet removed.
- All timings are from one machine, one developer, one day.
