# Usage

For the product overview, start with the [README](../README.md).

## Installing a release

The supported release target is Apple silicon macOS. The latest [swamp release](https://github.com/open-horizon-labs/swamp/releases/latest) contains `swamp` and `swamp-mcp`:

```bash
archive=swamp-aarch64-apple-darwin
curl -fLO "https://github.com/open-horizon-labs/swamp/releases/latest/download/$archive.tar.gz"
curl -fLO "https://github.com/open-horizon-labs/swamp/releases/latest/download/$archive.tar.gz.sha256"
shasum -a 256 -c "$archive.tar.gz.sha256"
tar -xzf "$archive.tar.gz"
mkdir -p ~/.local/bin
install -m 755 "$archive/swamp" "$archive/swamp-mcp" ~/.local/bin/
~/.local/bin/swamp --version
```

Release binaries are unsigned. If macOS blocks a downloaded binary with a quarantine warning, verify its checksum and source before deciding whether to clear that flag for the specific binary. You can also [build from source](../README.md#build-from-source).

## Observations and history

```bash
swamp report ~/src --since 24h
swamp report ~/src --since 7d --sort size --reverse
swamp observe ~/src
swamp schedule --every 15m ~/src
swamp schedule
swamp schedule --off
```

`report` measures and persists by default. `observe` records data without rendering and permits GitHub enrichment. Scheduling runs that observation command; it does not delete anything.

History starts when swamp observes a root. `--since` selects a comparison window, defaulting to the `since` config value. Use seconds, minutes, hours, or days here: `30m`, `24h`, `7d`. The filter language also accepts weeks, but the CLI/config history-duration parser does not; use `7d` rather than `1w` for `--since`.

`report --no-observe` skips persisting a new growth observation. It can still inspect the filesystem and consult enrichment caches; it is not a command for reading only cached JSON. The TUI's `--no-observe` uses its last cached report when one is available and disables the live watch.

Use `--full` to force a full filesystem walk. A normal observation can also fall back to a full walk when event history is insufficient; the report notes explain why.

The store currently partitions observations by volume. Prefer one common root such as `~/src`; use a separate `SWAMP_DIR` for independent roots on the same volume. See [storage limits](architecture.md#limits-of-the-current-implementation).

## Terminal controls

```bash
swamp ui ~/src
```

With no subcommand, `swamp` opens the UI at the current directory. It starts from a cached report when possible and refreshes in the background. The first observation can take longer.

| Key | Action |
|---|---|
| Up / Down | Move selection |
| Right / Left | Open or expand / collapse or return |
| Enter | Open a project or confirm the pending action |
| Esc | Cancel the current interaction or return to projects |
| Space | Mark or unmark a row |
| Backspace | Request removal of the selected row or marked set |
| `A` | Mark actionable rows in the current view, excluding the checkout fallback |
| `k` | Toggle keeping supported compiled outputs before removal |
| `/` | Open the filter form |
| `:` | Edit the filter expression; Tab completes terms |
| `0` | Clear the filter |
| `v`, `1`–`8` | Cycle/select projects, tree, builds, deps, Docker, kinds, unowned, types |
| `g`, `s`, `n`, `t`, `a` | Sort by growth, size, name, ecosystem, or age |
| `r` | Reverse the sort |
| `?` | Show help |
| `q` | Quit |

The initial filter is `growth > 100MB in 7d`. Filter, sort, reverse, and keep-executables choices are saved in `ui_state.json`. Clear the filter if the first observation shows no matching rows.

Rows show size and signed growth. Red bars extend right for increases; green bars extend left for decreases. Bar length is logarithmic, so use the number to compare exact changes.

## Report views

```bash
swamp report ~/src --all
swamp report ~/src --project api
swamp report ~/src --project api --dirs --depth 2
swamp report ~/src --view builds
swamp report ~/src --view rust
swamp report ~/src --view deps
swamp report ~/src --view types
swamp report ~/src --view docker
swamp report ~/src --view worktrees --filter 'merge-complete idle > 48h'
swamp report ~/src --view unowned
swamp report ~/src --view reconciliation --verify-du
```

Replace `api` with a project name from your report. Additional views include `kinds`; `--worktree <path>` prints one worktree's signals. The Rust view explains Cargo target/build storage as nested containers, profiles, dependencies, test/example outputs, build-script output, incremental state, final outputs, and companion metadata. Dependencies remain a folded directory aggregate, not a per-crate breakdown. Group sizes are allocated bytes; unknown subgroup hardlink charges are not reclaimable-space estimates. The view prints evidence limits and unknown variants. Final outputs are inspection-only.

Rust inspection does not invoke Cargo or build scripts. It reads layout and existing fingerprints; hashed filenames alone do not establish ownership, last execution, or obsolescence. In the TUI Builds view, mark an identified test/example executable or an individual incremental/build-script directory to review an exact cleanup group. CLI plans can select the same exact paths. Executable groups include existing dep-info and debug-symbol companions. Approval applies only to the reviewed group, not future files at that path.

Cleanup rechecks the group under Cargo's existing profile locks and moves it to a same-filesystem Trash envelope with a restore manifest. Changes since review require a new plan. Stop manual build writers first: Cargo locks are advisory. Missing locks, hardlinks, uncertain occupancy, incomplete scans, and unsupported layouts refuse cleanup. Shared dependency groups remain inspection-only; age alone never makes a group eligible.

The CLI currently applies `report --filter` only to the root `--view worktrees` output. It does not filter the builds view, project drill-down, overview, or JSON. For artifact filters use the TUI, MCP `report`, or `propose --filter`. `--project` scopes the project tree and supported artifact views; it does not scope every summary view.

`--json` returns the full report. Text-rendering options such as `--view`, `--project`, and `--depth` do not narrow that JSON:

```bash
swamp report ~/src --json --no-observe | jq '.summary.by_type'
swamp report ~/src --json --no-observe | jq '.reconciliation'
swamp report ~/src --json --no-observe |
  jq '[.projects[].worktrees[].artifacts[] | select(.growth_bytes > 1000000000) | {path, growth_bytes}]'
```

Filesystem reconciliation and Docker accounting are separate. `--verify-du` adds an independent `du -skPx` comparison and can take extra time.

## Filters

Supported interfaces share the core parser, but apply predicates to their own row types. Combine predicates with spaces:

Growth predicates filter the report's already-computed growth values. Their `in <duration>` clause does not currently recompute the baseline. For an explicit comparison, set CLI `--since` or MCP `since` to the same window. The TUI obtains its report window from configuration; changing the filter form's window can change the displayed label without changing those measurements. Use the CLI or MCP for an explicit window until that UI behavior is corrected.

| Expression | Meaning |
|---|---|
| `growth > 500MB in 7d` | Grew by more than the threshold; requires a report computed with the same window |
| `growth < 500MB in 7d` | Shrank by more than the threshold; not “grew by less than 500 MB” |
| `size > 1GB`, `size < 10MB` | Size threshold |
| `age > 30d` | Artifact's newest recorded modification is older than this; unknown age does not match |
| `idle > 48h` | Worktree idle time exceeds the threshold |
| `kind:BuildOutput`, `kind:deps` | Artifact kind by enum name or display label |
| `type:rust`, `type:js`, `type:python` | Ecosystem |
| `project:api`, `project:api-*` | Project name substring or glob |
| `merge-complete` | Combined branch-merge, clean, and unpushed facts |
| `pr:open`, `pr:merged`, `pr:closed`, `pr:none` | GitHub PR selection |

Filter durations include `30m`, `48h`, `7d`, and `1w`. Size units are decimal (`1MB = 1,000,000 bytes`); use `MiB` or `GiB` for binary units. Do not treat a `pr:none` match as proof of a successful GitHub lookup: unavailable facts can also lack a PR row.

## GitHub and Docker context

Install and authenticate `gh` to collect GitHub facts:

```bash
swamp observe ~/src
swamp report ~/src --view worktrees
swamp report ~/src --enrich --view worktrees
```

Plain reports use cached GitHub facts. `observe` and `--enrich` permit live queries; a valid cache entry can still be reused. GitHub cache validity uses the tip SHA and a six-hour TTL.

Docker facts are cached for five minutes, with fresh reads during enrichment. Docker must be installed and its daemon reachable. Unavailable Docker data is reported in notes.

Images, volumes, and build-cache records join to projects using Compose metadata or source-remote evidence. Unmatched objects remain unowned. Filesystem and Docker sizes should not be added to predict how much physical disk space an action will reclaim.

## Cleanup and recovery

In the UI, Space marks and Backspace asks for confirmation. Checkouts and linked worktrees can be selected as well as artifacts. Dirty, unpushed, and untracked warnings are shown for judgment; they do not universally block removal.

A project action expands to its actionable artifact rows. If it has none, a direct project action can offer the checkout. Bulk marking with `A` skips that fallback. The `ignored` and `untracked` remainder totals cover scattered files, so those summary buckets are not themselves deletion units.

The CLI separates proposal, approval, and execution:

```bash
swamp propose ~/src --filter 'kind:BuildOutput type:rust age > 30d'
```

Inspect the printed units and warnings, then use the returned ID:

```text
swamp approve <plan-id>
swamp execute <plan-id> --keep-executables
```

Proposing does not remove anything. CLI/MCP plans expire after 30 minutes and are single-use. Execution returns per-unit results; inspect refusals and failures as well as successful units.

For repeated work, a human can create and revoke a bounded standing grant:

```bash
swamp grant add 'kind:BuildOutput idle > 30d' --budget 5GB --expires 7d
swamp grant list
swamp plans
```

`swamp grant revoke <grant-id>` revokes it. Standing grants accept unit predicates, with required byte budget and expiry and optional `--max-units`. They reject growth-window and PR-state predicates.

| Unit | Removal and recovery |
|---|---|
| Filesystem path | Moved to Trash; swamp records its recovery location. Bytes remain on disk until the trashed data is removed. |
| Linked worktree or checkout | Can be moved to Trash through its specific action path. Inspect warnings about local work and repository context. |
| Docker image | Removed by Docker. Pulling or rebuilding depends on the image still being available or reproducible. |
| Docker volume | Removed by Docker. Swamp creates no copy of its contents. |
| Docker build-cache record | Reported, but individual removal is refused. |

`--keep-executables` copies supported Rust executables from `target/{release,debug}` and Python wheels/shared libraries from `dist` or `build` into the worktree's `bin/` before removal. It is not a backup of everything in the selected directory.

The ledger lives at `~/.local/share/swamp/ledger.jsonl`. Trashed bytes, permanent removals, and measured free-space change are different quantities. Consult the reported recovery location for restoration; swamp has no general undo command.

## Agent interface

Configure the stdio server as shown in the [README](../README.md#use-it-from-an-agent). Use absolute root paths in requests.

| Tool | Main inputs | Result |
|---|---|---|
| `report` | `root`, `since`, `project`, `view`, `filter`, `dirs` | Full report or named view |
| `what_grew` | Required `root` and `since` | Growing artifacts and coverage/history information |
| `list_projects` | `root`, `since` | Ranked project summaries |
| `list_worktrees` | `root`, `since`, `filter` | Worktree and GitHub facts |
| `docker_objects` | `root`, `project`, `unowned_only` | Docker objects and attribution |
| `propose` | Required `root`; optional `filter`, `paths`, `since` | Persisted action plan |
| `execute` | Required `plan_id`; optional `keep_executables` | Execution results for an authorized plan |
| `plans`, `grants` | None | Existing plans and grants |

Report tools can record new observations. They use cached GitHub facts and may refresh Docker's cache. Result metadata differs by tool; do not assume every response includes the same history fields.

Example tool-call payloads, with an illustrative root:

```json
{"name":"what_grew","arguments":{"root":"/Users/you/src","since":"7d"}}
```

```json
{"name":"report","arguments":{"root":"/Users/you/src","view":"builds","filter":"type:rust size > 1GB"}}
```

There is no MCP tool to create grants. Human authorization is supplied through the CLI or TUI. An agent with unrestricted shell access can also invoke the CLI, so this interface design is not an operating-system security boundary.

## Configuration

```bash
swamp config show
swamp config path
swamp config init
```

`config init` writes a file only if none exists. Defaults:

```toml
since = "24h"
retention_days = 30
large_file_min_bytes = 1048576
observe_timeout_sec = 1800
```

The file is `~/.local/share/swamp/config.toml`. `SWAMP_DIR` changes the store directory; give CLI, UI, and MCP the same value to share history. The schedule log defaults to `~/Library/Logs/swamp/observe.log`. The observation timeout applies to `observe`, not every interactive operation.

For a trace of stage timings:

```bash
SWAMP_TRACE=1 swamp report ~/src
```

See [architecture](architecture.md) for the meaning of incremental updates, history retention, and cached enrichment.
