# Usage

For the product overview, start with the [README](../README.md).

## Installing a release

On Apple silicon macOS, install with [Homebrew](https://brew.sh). The formula installs the `swamp` binary, the `skills/swamp/` agent skill, and verifies the release archive's checksum:

```bash
brew install open-horizon-labs/tap/swamp
swamp --version
```

Update with `brew upgrade swamp`; uninstall with `brew uninstall swamp`.
The tap checks for new releases every 15 minutes; GitHub may delay scheduled updates.

If you installed manually before, run `type -a swamp`. A copy in
`~/.local/bin` may take precedence over Homebrew. Remove that manually installed
copy after verifying `"$(brew --prefix)/bin/swamp" --version`.

Release archives remain available on the [releases page](https://github.com/open-horizon-labs/swamp/releases).
You can also [build from source](../README.md#build-from-source).

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

Observations are stored separately for each canonical scan root. You can switch between a project and its parent directory using the same `SWAMP_DIR`; each root keeps its own history and incremental checkpoint. Overlapping roots are separate views, not totals to add together.

## Scope and coverage

`report`, `observe`, `ui`, and `schedule` all take an explicit root. Omit
it and they resolve the same **effective scope**, computed by one shared
function so no command can silently disagree with another:

```bash
swamp scope
swamp scope --json
```

The effective scope is built from four sources, in this order:

1. **Built-in default roots** on macOS: `~/src`, `~/Library/Developer`,
   `~/Library/Caches`. (Linux has no built-in defaults yet.)
2. **Detector results.** A small built-in catalog of read-only
   detectors proposes locations for developer tools: Cargo home
   (`CARGO_HOME` or `~/.cargo`, split into the home directory, the
   registry cache, and the git-dependency cache), rustup
   (`RUSTUP_HOME` or `~/.rustup`), and Homebrew (`HOMEBREW_PREFIX`, or
   the conventional `/opt/homebrew` and `/usr/local` prefixes,
   optionally corroborated by a bounded, read-only `brew --prefix`
   call). A conventional path is still proposed even when the tool's
   executable is absent, so a leftover cache can still be found. This
   catalog will grow; `swamp scope --json` always lists the exact
   detector IDs and versions in use.
3. **`[scan] include`** in `config.toml`: extra roots always in scope.
4. **`exclude`** and **`disabled_detectors`**: pruned last, and always
   win over every other source -- including an explicit root you pass
   on the command line. Disabling a detector does not hide a path
   reachable through another still-enabled root (e.g. disabling the
   Cargo detector does not hide `~/.cargo` if it happens to live inside
   `~/src`, which is still in scope via the built-in defaults).

`[scan] defaults = false` turns off just the three built-in default
roots (source 1); every detector not separately named in
`disabled_detectors` still runs. This is "explicit-only scope": only
what you named in `include`, plus whatever detectors remain enabled.

Passing an explicit root (`swamp report ~/other-tree`) replaces sources
1-3 entirely for that invocation -- `exclude` still applies. A root that
sits inside another in-scope root is folded into its parent for
measurement (not walked twice); `swamp scope --json` still lists it,
marked `skipped-as-nested`, so you can see exactly why it did not get
its own line.

An effective scope that resolves to nothing at all -- `defaults =
false`, no `include`, and every detector disabled -- is a visible error,
never a silent fallback to the current directory or your home
directory. Malformed `[scan]` config (e.g. `defaults = "yes"` instead
of a bool) is also a visible, nonzero-exit error rather than a silently
broadened scope.

Coverage is not storage: adding a root, excluding one, or a detector
newly resolving a path is a change in what swamp *looks at*, not a
change in what exists on disk. `report`/`observe` persist the resolved
scope (`scope.json` under `SWAMP_DIR`) and print a one-line note on
stderr when it changes since the last observation, e.g. `coverage
changed since last observation: +root /Users/you/.cargo (detector
cargo-home), -root /Users/you/old-project (excluded)`. This note never
implies bytes were added or removed -- see
[coverage and history](../skills/swamp/references/coverage-and-history.md).

Full multi-root support differs by command today: `observe` and
`schedule` walk every present root in the effective scope (as many
roots as the scheduler already loops over). `report` and `ui` are
still single-root: with no explicit root they use the effective
scope's first present root and print a note if more than one is in
scope. Making `report`/`ui` themselves coverage-aware across every
resolved root is tracked separately (#42/#50); `swamp scope` and
`swamp observe`/`swamp schedule` already show/cover the full picture.

## Cleanup recommendations

Age is a cleanup signal, not a proof requirement. Supported Cargo cleanup groups
are ranked oldest-modified first, then largest when ages match. Missing or future
timestamps sort last. There is no minimum-age gate: recent builds remain reviewable.
The project tree and `cleanup-check` show modification age and rebuilding cost.
Modification age is not last execution or access time. Existing project/worktree
activity signals provide additional context; they are not required to suggest a
build cleanup candidate. Exact-selection checks and human approval still apply.

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

Rust inspection does not invoke Cargo or build scripts. It reads layout and existing fingerprints; hashed filenames alone do not establish ownership, last execution, or obsolescence. Opening a project in the TUI shows cleanup groups under each build profile: **Compiler caches**, **Compiled tests & examples**, and **Build-script output**, when supported members exist. Space marks a group's exact members for review; Backspace opens confirmation. Expand with → to choose Tests, Examples, or individual age-ranked members instead. Unrelated dependencies are not part of these groups. **Inspect directories** retains the physical layout as another view of the same bytes. No switch to Builds is required. The selected-row details explain cleanup recommendations and rebuilding consequences. Incremental compiler caches are suggested as a starting point if slower subsequent builds are an acceptable trade-off—not because Swamp has proved them obsolete. Compiled dependencies remain a folded aggregate without selective dependency cleanup.

In the project tree or Builds view, mark an identified test/example executable or an individual incremental/build-script directory to review an exact cleanup group. Physical category rows only expand; purpose-based cleanup groups in the project tree mark their supported members. CLI plans can select the same exact paths. Executable groups include existing dep-info and debug-symbol companions. Approval applies only to the reviewed group, not future files at that path.

### Build details: choose what to give up

Open a project with → to see cleanup groups beneath each Cargo build profile. Choose by the cost of rebuilding, then expand a group if you want to remove only older members.

![Swamp's project tree showing debug compiler caches, compiled tests and examples, and build-script output, with sizes, removal consequences and an oldest-candidate preview.](images/cargo-build-cleanup.png)

Its sizes and ages are one observation of Swamp's own build directory, not expected savings for every project. Old `slop_livin` names are build artifacts left from the project's earlier name.

| Group | In this screenshot | What removal changes |
|---|---|---|
| Compiler caches | 11.7 GB allocated, 617 groups | Discards incremental compiler state. Start here if a slower subsequent build is acceptable. |
| Compiled tests & examples | 8.8 GB allocated, 206 groups | Removes identified test executables and examples. Rebuild before rerunning; unrelated compiled dependencies are not selected. |
| Build-script output | 123.6 MB allocated under debug | Scripts run again on a later build and may need external tools or network access. |
| Inspect directories | Another view of the profile's bytes | Shows the physical layout, including dependencies, final outputs and metadata. It is not another cleanup group or additional storage. |

**Choose a group or individual members.** Space marks a cleanup group's exact supported members. → expands it; Compiled tests & examples splits into Tests and Examples, then individual members ordered by modification age. Backspace opens review for the marked selection, Enter confirms, and Esc cancels confirmation. Marking a fully marked group clears its marks. A failed member check rolls back newly added marks rather than silently selecting only part of the group. Stop builds before cleanup; marking can take time because it checks the selected contents.

**Read age as a suggestion, not proof of disuse.** `Oldest 3d` means the oldest known modification age among the group's candidates, not that every member is three days old or has gone unused for three days. Expand to choose older members; selecting the collapsed group includes recent members too. Unknown age appears as `?`. The lower preview shows only the stated subset—31 of 617 in this screenshot—not the full selection.

**Do not add all the displayed sizes.** A `*` marks allocated bytes, which can count shared hardlinks more than once. That is why debug can show 34.5 GB while the containing build target shows 30.1 GB on a different accounting basis. Parent rows include their children, and Inspect directories repeats the same storage by path. A dash in a purpose group's Change column means no aggregate growth value is supplied, not zero growth.

**Candidates are not guaranteed free space.** The debug profile's 20.7 GB candidate total covers supported cleanup members, not all debug output. Missing groups or “Selective cleanup unsupported” describe Swamp's action support, not a requirement to retain those files. Final outputs and the remaining compiled dependencies are inspection-only for selective cleanup. Space or Backspace on a profile reviews all supported cleanup groups beneath it, not the entire profile directory. Use the candidate total, not the profile's full size, to understand that selection.

Cleanup moves supported filesystem groups to Trash; those bytes are not immediately freed. Emptying Trash later may reclaim space, but surviving hardlinks and filesystem snapshots can limit the result. Source files and unrelated dependency artifacts are outside these purpose-based selections. Existing identity, occupancy, Cargo-lock and approval checks still apply.

### Progress and cancellation

Marking runs review checks in the background. After you confirm, a **Deleting**
bar shows processed/total groups, successful and refused counts, elapsed time,
and the current path. It measures groups processed, not bytes freed. Review and
deletion keep the terminal responsive; additional actions wait until they finish.

**Esc or Ctrl-C stops after the current group.** Swamp finishes that group's
check or move and records its outcome before stopping. Completed moves remain in
Trash; refused and unattempted selections remain marked for explicit review or
retry. Cancelling review preserves the selection you had before review started
and does not delete anything. Ctrl-C exits when no operation is running.

### Review exact build groups from the CLI

The Rust text view shows the largest 30 rows by default; add `--all` for the full list. Category totals include their children: do not sum them. A category is not an individual cleanup selection. `unchecked` means checks have not run, not that the group is unused. Report JSON includes the same guidance under each nested row's `cleanup` field.

Review a bounded selection before deciding what to remove:

```sh
swamp cleanup-check ~/src/my-project --role test-executable --limit 3
swamp cleanup-check ~/src/my-project --role incremental --limit 5 --json
swamp cleanup-check ~/src/my-project --role incremental --limit 5 --offset 5 --json
swamp cleanup-check ~/src --within ~/src/my-project/target --role incremental --limit 5
swamp cleanup-check ~/src/my-project --path /absolute/path/to/target/debug/incremental/crate-group
```

This command observes the root, then checks selected groups (five by default, at most twenty). Checks can read the group's contents and create **unapproved** plans; they never authorize or execute cleanup. Results distinguish `blocked`, `unchecked`, and `ready_for_review`, with reason codes, exact members for successful checks, recovery details and timings. A reviewed group is not confirmed unused. A failed group never expands into removal of its parent.

The result shows the number and allocated size of all observed candidates in scope, how many were not checked in this run, and arguments for the next age-ranked page. It also counts coverage-limited and unidentified Cargo rows separately; zero candidates does not establish that there is no cleanup opportunity. These coverage counts ignore `--role`, because an unknown row cannot reliably match a requested role. Pages can shift if builds change between calls. `--within` narrows candidate discovery to groups strictly below a directory; use `--path` to review that exact directory if it is a selectable group. A five-group result is not a measure of the total cleanup opportunity, and candidate bytes are not a promise of reclaimable space.

Hardlinked groups can be reviewed and moved to Trash. Links outside the selected group remain intact; reclaimable space is unknown. Lock failures distinguish unavailable locks from missing lock files; retry after builds finish, not by widening scope. Allocated bytes are not promised free space, and moving files to Trash does not free those bytes immediately. JSON `next_command` and `next_page` are argument arrays, not shell strings; retain the same `SWAMP_DIR` to find the created plans.

Cleanup rechecks the group under Cargo's existing profile locks and moves it to a same-filesystem Trash envelope with a restore manifest. Changes since review require a new plan. Stop manual build writers first: Cargo locks are advisory. Missing locks, uncertain occupancy, incomplete scans, and unsupported layouts refuse cleanup. Shared dependency groups remain inspection-only; age alone never makes a group eligible.

The CLI's *text* rendering applies `--filter` only to the root `--view worktrees` output; it does not filter the builds view, project drill-down, or overview text. For a filter that narrows every row, add `--json`: see below and [the agent interface](#agent-interface).

`--json`, with or without `--view`, applies `--filter` (when given) to the whole report before computing any output, and scopes to `--project` when given -- narrower than an unfiltered dump and consistent between the full report and every named view:

```bash
swamp report ~/src --json --no-observe | jq '.summary.by_type'
swamp report ~/src --json --no-observe | jq '.reconciliation'
swamp report ~/src --view grown --json --since 24h --filter 'kind:BuildOutput' | jq '.result.grown'
swamp report ~/src --json --no-observe |
  jq '[.projects[].worktrees[].artifacts[] | select(.growth_bytes > 1000000000) | {path, growth_bytes}]'
```

Large results can be bounded with `--limit`/`--offset`; the envelope's `total`/`truncated` fields say whether a page is the whole answer. See `skills/swamp/references/commands-and-json.md` for the full schema.

Filesystem reconciliation and Docker accounting are separate. `--verify-du` adds an independent `du -skPx` comparison and can take extra time.

## Filters

Supported interfaces share the core parser, but apply predicates to their own row types. Combine predicates with spaces:

Growth predicates filter the report's already-computed growth values. Their `in <duration>` clause does not currently recompute the baseline. For an explicit comparison, set CLI `--since` to the same window. The TUI obtains its report window from configuration; changing the filter form's window can change the displayed label without changing those measurements. Use the CLI for an explicit window until that UI behavior is corrected.

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
swamp propose ~/src --path /absolute/path/to/a-worktree
```

For a worktree, first inspect `swamp report <root> --view worktrees` for dirty, unpushed and merge evidence. Use its exact reported path in `propose --path`; the scan root must cover that worktree. You can use the same store when switching between a project root and a parent containing related worktrees.

Inspect the printed units and warnings, then use the returned ID:

```text
swamp approve <plan-id>
swamp execute <plan-id> --keep-executables
```

Proposing does not remove anything. Plans expire after 30 minutes and are single-use. Execution returns per-unit results; inspect refusals and failures as well as successful units.

The plan's `created_at` is its review time. A unit's `observed_at` can be older when incremental replay reused an unchanged measurement; it is not restamped to pretend the data was remeasured.

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

Through v0.6.x, agent access went through a separate `swamp-mcp` stdio
server. That server is removed (#104): the CLI's `--json` output is now
the sole supported machine interface, and `skills/swamp/` packages it
as an installable agent skill. Use absolute root paths in commands.

### Installing the skill

Copy or symlink the skill directory into your agent client's skills
location, without editing any other client configuration:

```bash
# Claude Code project-scoped skill, from a checkout of this repo:
mkdir -p .claude/skills
ln -s /absolute/path/to/swamp/skills/swamp .claude/skills/swamp

# Or copy it in (e.g. for a user-level skills directory some clients read):
cp -R /absolute/path/to/swamp/skills/swamp ~/.claude/skills/swamp
```

Consult your specific agent client's own documentation for where it
looks for skills; swamp does not assume or silently modify a client's
configuration file to register one. A release archive built from this
repository includes `skills/swamp/` alongside the `swamp` binary so
both ship together.

### The JSON contract

Every command below is noninteractive: it prints exactly one JSON
document to stdout (with `--json`), diagnostics on stderr, a
deterministic schema, and documented exit codes. Full schemas, the
historical MCP-tool-to-CLI mapping, pagination, and the exit-code
contract: `skills/swamp/references/commands-and-json.md`.

| Command | Main flags | Result |
|---|---|---|
| `report <root> --json` | `--since`, `--project`, `--view`, `--filter`, `--dirs`, `--limit`/`--offset` | Full report or named view, bounded |
| `report <root> --view grown --json` | Required `--since` for a meaningful window | Growing artifacts plus coverage/history information |
| `report <root> --view projects --json` | `--since` | Ranked project summaries |
| `report <root> --view worktrees --json` | `--filter` | Worktree and GitHub facts |
| `report <root> --view docker --json` | `--project`, `--unowned-only` | Docker objects and attribution |
| `propose <root> --json` | `--filter`, `--path`, `--since` | Persisted action plan, `awaiting-authorization` |
| `execute <plan_id> --json` | `--keep-executables` | Execution results for an authorized plan |
| `plans --json`, `grant list --json` | None | Existing plans and grants |

`report --json` can record new observations (skip with `--no-observe`).
It uses cached GitHub facts and may refresh Docker's cache. Result
metadata differs by view; do not assume every response includes the
same history fields -- check `skills/swamp/references/commands-and-json.md`.

Example calls, with an illustrative root:

```bash
swamp report /Users/you/src --view grown --json --since 7d
swamp report /Users/you/src --view builds --json --filter 'type:rust size > 1GB'
```

There is no CLI command that both an agent and a human can use to mint
authorization silently -- `swamp approve`/`swamp grant add` exist for a
human to run. An agent with unrestricted shell access can invoke those
same commands, so this is a followed convention, not an
operating-system security boundary. See
[the trust model](../skills/swamp/references/trust-model.md) for what
actually enforces safety (sink re-derivation, scoped/budgeted/expiring
grants, the ledger).

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

[scan]
defaults = true
include = []
exclude = []
disabled_detectors = []
```

`config init`'s `[scan]` table is not a frozen copy of the built-in
default roots or the detector catalog -- it documents the four keys
with their meaning; the actual defaults and detector catalog live in
the binary and can grow across releases without editing every user's
config. See [Scope and coverage](#scope-and-coverage) for what each key
does and `swamp scope --json` for the resolved result. `config show`
and `config init` both refuse (nonzero exit, message on stderr) on a
`config.toml` with a malformed `[scan]` table, rather than silently
falling back to the all-defaults scope.

The file is `~/.local/share/swamp/config.toml`. `SWAMP_DIR` changes the store directory; give the CLI and UI the same value (interactively or from an agent's `--json` calls) to share history. The schedule log defaults to `~/Library/Logs/swamp/observe.log`. The observation timeout applies to `observe`, not every interactive operation.

For a trace of stage timings:

```bash
SWAMP_TRACE=1 swamp report ~/src
```

See [architecture](architecture.md) for the meaning of incremental updates, history retention, and cached enrichment.
