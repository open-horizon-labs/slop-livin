# slop-livin

Find out what grew on your disk, by project, and delete it from where you're standing.

`slop-livin` is for developers who run many coding agents at once. Every agent builds, installs, and caches, and a week later the disk is full and nobody remembers which project did it. `du` tells you what is big. `slop-livin` tells you what **grew**, attributes it to a git project → checkout/worktree → artifact, and lets you (or your agent) act on it with the facts in front of you.

```
/Users/me/src · observed 2m ago · since 24h · 56 projects · 41.3GB attributed · 202.9MB unowned · docker 15.8GB unowned
view: tree of open-horizon-labs/governed-compositional-etl  (Esc back) · filter: 0
└─ ▾ main .../governed-compositional-etl        2.0GB   +380.9KB   last commit 1h · dirty · 26 unpushed
   ├─ git .git                                  5.6MB   +217.1KB
   ├─ ▾ source .  [tracked]                     1.1GB   +139.3KB
   │     ├─ dir raw  [untracked]                1.1GB
   │     ├─ dir chain  [tracked]                3.3MB
   │     └─ … and 10 more directories         745.5KB
   ├─ build build  [ignored]                  267.0MB    +24.6KB
   ├─ cache .cache  [ignored]                 290.9MB         0B
   └─ deps .venv  [ignored]                   251.5MB         0B
```

That `raw/` line is the point. It is 1.1 GB, it is not in git, and no `.gitignore` covers it. Nothing would bring it back. The tool says so before you press Backspace; it does not decide for you.

macOS only for now (FSEvents, LaunchAgent, Trash).

## Install

```bash
git clone https://github.com/open-horizon-labs/slop-livin
cd slop-livin
cargo build --release
cp target/release/slop-livin target/release/slop-livin-mcp ~/.local/bin/
```

Optional: observe every 15 minutes in the background so growth history accumulates without you running anything.

```bash
slop-livin schedule --every 15m ~/src
```

`slop-livin schedule --off` removes it. It is a per-user LaunchAgent (`ProcessType Background`, low I/O priority); the log is at `~/Library/Logs/slop-livin/observe.log`.

## Usage

### Terminal UI

```bash
slop-livin ui ~/src
```

Opens in milliseconds from the last observation and refreshes in the background. Rows are `git diff --stat` lines: bytes, signed growth, a `+`/`-` bar scaled to the visible set, then facts.

| Key | Does |
|---|---|
| `↑` `↓` | move |
| `→` `←` | expand / collapse a worktree or a `source` row |
| `Enter` | open a project (in the projects view); confirm a delete |
| `Space` | mark / unmark the row |
| `⌫` | delete what is under the cursor (or the marked rows), after one confirm |
| `/` | filter form: growth, window, kind, project, idle, merge-complete, PR, type, size, age |
| `:` | filter as text, with Tab completion |
| `v` `1`–`8` | views: projects · tree · builds · deps · docker · kinds · unowned · types |
| `g` `s` `n` `t` `a` | sort by growth / size / name / type / age; `r` reverses |
| `k` | keep executables: before trashing `target/` copy its `release`/`debug` binaries to `bin/`; `dist/*.whl` and `build/**/*.so` likewise |
| `Esc` | back to projects |
| `?` | help |

The confirm line states what you are about to do and what the tool knows about it:

```
delete governed-compositional-etl ⚠ dirty · 26 unpushed · raw untracked 1.1GB (2.0GB) → Trash?  Enter yes · Esc no
```

Everything goes to Trash. The ledger (`~/.local/share/slop-livin/ledger.jsonl`) records what was removed, by whom, and which warnings were on screen at the time.

Filter and sort persist between sessions. The growth window can only be as long as the history the store actually holds; the header says `since 4h (asked 1w; history is 4h)` rather than pretending.

### Command line

```bash
slop-livin report ~/src                                   # one screen: projects by growth, then size
slop-livin report ~/src --project mole                    # tree: checkouts → worktrees → artifacts
slop-livin report ~/src --project mole --view builds      # or deps, docker, kinds, unowned, worktrees
slop-livin report ~/src --view worktrees --filter 'merge-complete idle > 48h'
slop-livin report ~/src --view reconciliation             # attributed + unowned = walked, du if asked
slop-livin report ~/src --json
```

Every project row wears its ecosystems as glyphs, from the markers at the checkout root: 🦀 Rust · ⬢ Node · 🦕 Deno · 🐍 Python · 🐹 Go · ☕ JVM · 🔺 Scala · 🔧 C/C++ · 🐦 Swift · 🟣 .NET · 💎 Ruby · 💧 Elixir · 🐘 PHP · λ Haskell · 🎯 Dart · ⚡ Zig · 🌍 Terraform · 🐳 Docker · 🎲 Unity · 🎮 Unreal, plus 🔨 when it holds build output and ⎇N for N linked worktrees. A checkout can be several at once.

```bash
slop-livin report ~/src --sort size --reverse             # smallest first; also name, type, age
slop-livin report ~/src --view types                      # per ecosystem: projects, artifacts, bytes, growth
slop-livin execute <plan> --keep-executables              # copy compiled outputs to bin/ before trashing
slop-livin config show | path | init                      # the config file, spelled out
```

Filter grammar, shared by the CLI, the TUI and the MCP tools: `growth > 500MB in 30d` · `size > 500MB` · `age > 30d` · `kind:BuildOutput` · `project:mole` (or a glob, `project:my-app*`) · `type:rust` · `idle > 48h` · `merge-complete` · `pr:open|merged|closed|none`. Sizes are decimal (`500MB` is 500,000,000 bytes, the same base the tool prints in); write `500MiB` for binary. `age` is time since an artifact was last written; `idle` is time since a worktree's last commit or edit.

### Agent (MCP)

`slop-livin-mcp` speaks MCP over stdio. It is the primary operator surface: an agent can ask what grew, propose a plan, and execute it once a human has authorized it. It cannot authorize anything itself.

| Tool | Answers |
|---|---|
| `report` | the full report, or a named `view`, scoped to a `project`, with `dirs:true` for per-directory rollups |
| `what_grew` | rows that grew in the window, sorted, plus coverage and the history the store holds |
| `list_projects` | ranked projects: `owner/repo`, bytes, growth, checkout and worktree counts |
| `list_worktrees` | branch, idle time, merge-complete with its terms, PR state |
| `docker_objects` | every image / build-cache entry / volume with created date, containers, shared layers |
| `propose` | a plan from any paths in the report; every unit carries bytes, growth, recovery, git status and warnings |
| `execute` | run an authorized plan: Trash, ledger, measured free space; refused units name the fact |
| `plans` `grants` | read-only listings |

Authorization is a human at the keyboard:

```bash
slop-livin approve <plan-id>                                             # this plan, once; prints each unit's warnings first
slop-livin grant add 'kind:BuildOutput idle > 30d' --budget 5GB --expires 7d   # standing, bounded
```

There is no MCP tool that writes a grant. That is the design, not an omission.

## What it knows

**Projects, not paths.** A project is its git object store, unified across clones by remote, so two checkouts of `roon-knob` in different directories are one project with two checkouts and their linked worktrees. Rows are named `owner/repo`.

**Project types from markers, artifacts from types.** `Cargo.toml` makes a checkout Rust, `package.json` Node, `pyproject.toml` Python, and so on for twenty ecosystems; the same table says which directories each one generates. Unambiguous names (`node_modules`, `target`, `.venv`, `.stack-work`) are artifacts anywhere. Names several tools use (`build`, `dist`, `vendor`, `bin`, `obj`, `out`, `*.egg-info`) count only next to a marker of an ecosystem that generates them, so a repo's hand-written `build/` directory is source, and `.NET`'s `bin/` is build output. This is clean-dev-dirs' per-language detection, kept as a table so the CLI, the TUI, the MCP server and the filter all read one truth. A checkout with no remote is named from its manifest (`[package] name`, `"name"`, `module`, `<artifactId>`, …) instead of its directory.

**Artifacts as units.** `node_modules`, `target`, `build`, `dist`, `.venv`, `.cache`, `.git` and friends are folded: sized as one unit, never descended, attributed to the nearest containing checkout. Deleting one is one action. Everything that isn't an artifact is the worktree's `source`, which expands into its own directories.

**Git status on every row.** `tracked`, `ignored`, or `untracked` — from git's own exclude rules via gitoxide, verified against `git check-ignore`. Untracked bytes are the ones no clone brings back.

**Activity as facts.** Last commit age, dirty, unpushed count, locked, idle time, computed in-process at walk time. With `gh` available, every GitHub-remote worktree is enriched with its PR (number, state, review) and whether the branch is merged into the default branch, one GraphQL call per repository, cached six hours. The composite `merge-complete` is always shown with its terms; the tool never says "safe" or "stale".

**Docker as part of the same answer.** Images, build cache and volumes are joined to projects only on explicit evidence: a compose label, a compose file's `name:` inside a worktree, or `org.opencontainers.image.source` matching a remote. Everything else is listed as unowned with the reason. Name similarity never attributes. Docker bytes are reported next to, not inside, the filesystem total.

**Honest totals.** `attributed + unowned = walked`, to the byte, on every report. `--verify-du` runs `du -skPx` as an independent oracle. Coverage names permission-denied directories rather than hiding them.

## How it works

**Column store with reverse deltas.** Each observation writes the current state to Parquet (zstd) plus an append-only delta holding the *previous* values of rows that changed. Growth over any window is a lookup, not a rescan; a no-change observation appends nothing; deltas compact and age out on a configurable retention window. On the test machine the store for 56 projects, 107 worktrees and 25 k directory rollups is a few hundred KiB.

**Directory folding keeps it small.** There are no per-file rows. Artifact trees are one row each; Source trees get per-directory rollups (with `mod_time_min` as int32 minutes) and rows only for files over 1 MiB.

**FSEvents-driven incremental walks.** The stream id is stored with each observation. The next observation replays events since that id, re-sizes only the changed subtrees, and carries every other row forward; hardlinks are accounted per row so a re-size can never double-count. It refuses and falls back to a full walk on any doubt (no stored id, id from the future, history exhausted, too many changes), and says which. Full walk of ~41 GB / 107 worktrees: ~7.5 s. Incremental: 2–3 s. The result matches a forced full walk to the byte.

**Parallel walk.** Directory reads run on a bounded pool, one `lstat` per entry, staying on one device, never following symlinks.

**One report, three surfaces.** The TUI, CLI and MCP render the same `Report`. Nothing is computed only for the screen.

**Statically checked invariants.** A `syn`-based audit fails the build on a second byte formatter, an uncompressed Parquet writer, an unbounded join, or verdict vocabulary in output.

## Compared with

| | slop-livin | [kondo](https://github.com/tbillington/kondo) | [npkill](https://github.com/voidcosmos/npkill) | [clean-dev-dirs](https://github.com/clean-dev-dirs/clean-dev-dirs) | [cargo-sweep](https://github.com/holmgr/cargo-sweep) | [Mole](https://github.com/tw93/Mole) | StorageRadar | DaisyDisk |
|---|---|---|---|---|---|---|---|---|
| Finds build/deps artifacts | ✓ ~50 names, 20 ecosystems, marker-gated where the name is ambiguous | ✓ 20+ types | node_modules | ✓ 16 ecosystems | Cargo `target/` | ✓ (purge) | | |
| Project type on every row, filter and sort by it | ✓ glyph badges, `type:` filter, `t` sort, per-type view | ✓ tag | | ✓ tag, `-p`, `--sort type` | | | | |
| Size / age thresholds | ✓ `size >`, `age >`, in TUI form and filter | | ✓ | ✓ `--keep-size`, `--keep-days` | ✓ `--time` | | | |
| Keep compiled outputs before deleting | ✓ `k` / `--keep-executables` (Rust, Python) | | | ✓ `-k` (Rust, Python) | | | | |
| Growth over time | ✓ persistent, any window | | | | | | manual snapshots | |
| Project → worktree → artifact model | ✓ | project | | project | | | directories | directories |
| Git status per row (tracked/ignored/untracked) | ✓ | | | | | | | |
| Worktrees, merged-branch, PR state | ✓ | | | | | | | |
| Docker attributed to projects | ✓ | | | | | | | |
| Agent interface | MCP, propose/execute with human grant | | | | | | read-only MCP | |
| Incremental observation | FSEvents | | | | | | | |
| Deletes to | Trash | rm | rm | Trash | rm | Trash | | Trash |
| Price | free | free | free | free | free | free (Mac app paid) | $19.99 | $9.99 |

kondo and clean-dev-dirs are the right tool when you want a one-shot sweep of build output across ecosystems and don't need history. cargo-sweep is the right tool inside a single Rust workspace. Mole is a general macOS cleanup toolkit; `slop-livin` started as a Mole contribution and became its own thing when the questions turned out to be about projects, not caches. StorageRadar and DaisyDisk are directory-tree visualizers; DaisyDisk has no history, StorageRadar's is manual snapshots without a project model.

## Numbers, and their limits

Everything above was measured on one machine, one developer, over one day: 56 projects, 107 worktrees, ~41 GB under `~/src`, 270 Docker objects. Treat the timings as one data point, not a benchmark.

## Design notes

`PRODUCT.md` and `DESIGN.md` carry the product record and the TUI's design contract. `.oh/sessions/` holds the reasoning: aim, problem space, the salvage of the first attempt, and the plan. The short version: the unit of decision is the project, activity is the deciding attribute, and the tool states facts while the human decides.
