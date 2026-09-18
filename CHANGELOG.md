# Changelog

Release notes live here, one section per tag. The release workflow refuses a tag without one.

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
