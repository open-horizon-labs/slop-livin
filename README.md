# slop-livin

Find out what grew on your disk, by project, and delete it from where you're standing.

`slop-livin` is for developers who run many coding agents at once. Every agent builds, installs, and caches, and a week later the disk is full and nobody remembers which project did it. `du` tells you what is big. `slop-livin` tells you what **grew**, attributes it to a git project → checkout/worktree → artifact, and lets you (or your agent) act on it with the facts in front of you.

```
~/src · observed just now · 56 projects · 44.7GB attributed · 202.9MB unowned · docker 15.8GB unowned
view: projects · filter: none · sort: growth
open-horizon-labs/slop-livin  🦀🔨                      16.3GB    +10.5GB ██████████████████████   ▂   ▁▂  ▇ █
open-horizon-labs/unified-hifi-control  🦀⬢🐍🔨⎇1        5.5GB     -1.5GB ███▎                             █
open-horizon-labs/governed-compositional-etl  🔨         2.0GB   +266.6MB ▌                                 █
muness/roon-knob  🔨                                     5.2GB   +221.5MB ▌                                 █
open-horizon-labs/northwoods  ⬢🐳🔨                      1.2GB   -126.8MB ▎
```

Each row is a `git diff --stat` line for a project: size, then the signed change over the window with its bar (red grew, green shrank), then *when* it moved, then facts. Enter opens the project:

```
view: tree of governed-compositional-etl  (Esc back) · filter: none
└─ ▾ main ~/src/open-horizon-labs/governed-compositional-etl        2.0GB   +266.6MB ██████████████████████
   ├─ ▸ source .  [tracked]                                          1.4GB   +266.3MB ██████████████████████
   │     ├─ dir raw  [untracked]                                     1.1GB
   │     └─ … 10 more directories
   ├─ git .git                                                       5.6MB   +217.1KB ▏
   ├─ cache .cache  [ignored]                                      290.9MB         0B
   ├─ deps .venv  🐍  [ignored]                                    251.5MB         0B
   └─ artifacts scripts/__pycache__  [ignored]                     274.4KB         0B
```

That `raw/` line is the point. It is 1.1 GB, it is not in git, and no `.gitignore` covers it. Nothing would bring it back. The tool says so before you press Backspace; it does not decide for you.

macOS only for now (FSEvents, LaunchAgent, Trash).

## Install

Apple silicon macOS. Each release ships a tarball and its checksum.

```bash
v=0.2.0
curl -fsSLO "https://github.com/open-horizon-labs/slop-livin/releases/download/v$v/slop-livin-$v-aarch64-apple-darwin.tar.gz"
curl -fsSLO "https://github.com/open-horizon-labs/slop-livin/releases/download/v$v/slop-livin-$v-aarch64-apple-darwin.tar.gz.sha256"
shasum -a 256 -c "slop-livin-$v-aarch64-apple-darwin.tar.gz.sha256"
tar xzf "slop-livin-$v-aarch64-apple-darwin.tar.gz"
xattr -d com.apple.quarantine "slop-livin-$v-aarch64-apple-darwin"/slop-livin*   # binaries are unsigned
install -m 755 "slop-livin-$v-aarch64-apple-darwin"/slop-livin* ~/.local/bin/
slop-livin --version
```

Or from source (Rust 1.92+):

```bash
git clone https://github.com/open-horizon-labs/slop-livin && cd slop-livin
cargo build --release
install -m 755 target/release/slop-livin target/release/slop-livin-mcp ~/.local/bin/
```

## Quick start

```bash
slop-livin ui ~/src                       # the TUI: what grew, by project; Backspace deletes to Trash
slop-livin report ~/src                   # the same on one screen
slop-livin report ~/src --view types      # by ecosystem: Rust 20GB, Node 5.6GB, …
slop-livin schedule --every 15m ~/src     # observe in the background so growth has history
claude mcp add slop-livin ~/.local/bin/slop-livin-mcp   # let your agent ask instead of you
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

Opens in milliseconds from the last observation and refreshes in the background; the header shows `observing… 12.3GB · 4210 dirs · 31%` while it does.

| Key | Does |
|---|---|
| `↑` `↓` | move |
| `→` `←` | expand / collapse a worktree or a `source` row |
| `Enter` | open a project (in the projects view); confirm a delete |
| `Space` | mark / unmark the row |
| `⌫` | delete what is under the cursor (or the marked rows), after one confirm |
| `k` | keep executables: before trashing `target/`, copy `release/`/`debug/` binaries to `bin/` (Python: `dist/*.whl`, `build/**/*.so`) |
| `/` | filter form: growth, window, kind, project, idle, merge-complete, PR, type, size, age |
| `:` | filter as text, with Tab completion |
| `0` | clear the filter |
| `v` `1`–`8` | views: projects · tree · builds · deps · docker · kinds · unowned · types |
| `g` `s` `n` `t` `a` | sort by growth / size / name / type / age; `r` reverses |
| `Esc` | back to projects |
| `?` | help |

**Columns.** bytes · signed growth in the window with its bar, red for bytes arriving and green for bytes leaving, scaled to the largest change on screen · a sparkline of *when* it moved, one bar per time bucket, `·` before the row was first observed · facts (`last commit 1h · dirty · 26 unpushed`, `[tracked]` / `[ignored]` / `[untracked]`).

**Badges** after a project's name are its ecosystems, from the markers at the checkout root: 🦀 Rust · ⬢ Node · 🦕 Deno · 🐍 Python · 🐹 Go · ☕ JVM · 🔺 Scala · 🔧 C/C++ · 🐦 Swift · 🟣 .NET · 💎 Ruby · 💧 Elixir · 🐘 PHP · λ Haskell · 🎯 Dart · ⚡ Zig · 🌍 Terraform · 🐳 Docker · 🎲 Unity · 🎮 Unreal, plus 🔨 when it holds build output and ⎇N for N linked worktrees. A checkout can be several at once.

**Deleting.** Backspace asks once, and the question states what the tool knows:

```
delete governed-compositional-etl ⚠ dirty · 26 unpushed · raw untracked 1.1GB (2.0GB) → Trash?  Enter yes · Esc no · k keep executables
```

Everything goes to Trash. The row and everything under it leave the screen at once, the totals drop by what left, and an incremental re-observe runs in the background. The ledger (`~/.local/share/slop-livin/ledger.jsonl`) records what was removed, by whom, and which warnings were on screen.

Filter, sort and the keep-executables setting persist between sessions. The growth window can only be as long as the history the store holds; the header says `since 4h (asked 1w; history is 4h)` rather than pretending.

### Command line

Every question is one command. `report` observes (and records) by default; add `--no-observe` to read the last observation.

```bash
# What grew, by project (growth, then size). --all for every row.
slop-livin report ~/src
slop-livin report ~/src --since 7d --sort size --reverse         # window; smallest first (also name, type, age)

# One project: checkouts → worktrees → artifacts → directories
slop-livin report ~/src --project roon-knob
slop-livin report ~/src --project roon-knob --dirs --depth 2    # per-directory growth, large files that grew

# Views, at the root or narrowed with --project
slop-livin report ~/src --view types                            # per ecosystem: projects, artifacts, bytes, growth
slop-livin report ~/src --view builds                           # every build output / cache, largest first
slop-livin report ~/src --view deps                             # every dependency tree
slop-livin report ~/src --view worktrees --filter 'merge-complete idle > 48h'   # worktrees whose branch is merged and idle
slop-livin report ~/src --view docker                           # images, build cache, volumes, joined or unowned
slop-livin report ~/src --view unowned                          # bytes under the root that no project claims
slop-livin report ~/src --view reconciliation --verify-du       # attributed + unowned = walked, checked against du

# Filters compose (all must hold)
slop-livin report ~/src --view builds --filter 'type:rust size > 1GB age > 30d'
slop-livin report ~/src --view worktrees --filter 'project:hiphi-* pr:merged'

# Acting: propose → a human approves → execute
slop-livin propose ~/src --filter 'kind:BuildOutput type:rust age > 30d'
slop-livin approve <plan-id>                                     # prints each unit's warnings first
slop-livin execute <plan-id> --keep-executables                  # Trash; binaries copied to <worktree>/bin/ first
slop-livin grant add 'kind:BuildOutput idle > 30d' --budget 5GB --expires 7d   # a standing, bounded grant
slop-livin plans                                                  # what was proposed, by whom, what happened

# Plumbing
slop-livin observe ~/src                                          # observe only (what the schedule runs); refreshes GitHub facts
slop-livin schedule --every 15m ~/src                             # LaunchAgent; --off removes it
slop-livin config show | path | init                              # the config file, every key with its meaning
slop-livin report ~/src --json                                    # the full report as JSON
```

**Filter grammar**, shared by the CLI, the TUI form and line, the MCP tools and grants:

| Predicate | Matches |
|---|---|
| `growth > 500MB in 30d` · `growth < 1GB in 7d` | change over the window (capped at the history the store holds) |
| `size > 500MB` · `size < 10MB` | the unit's bytes; a project's or worktree's total on rollup rows |
| `age > 30d` | time since an artifact was last written (unknown never matches) |
| `idle > 48h` | time since a worktree's last commit or edit |
| `kind:BuildOutput` · `kind:deps` | artifact kind, enum name or label |
| `type:rust` · `type:js` · `type:python` | ecosystem tag or name |
| `project:mole` · `project:my-app*` | substring, or a glob with `*` and `?` |
| `merge-complete` | branch merged into the default branch, clean, nothing unpushed |
| `pr:open` · `pr:merged` · `pr:closed` · `pr:none` | GitHub PR state (needs `gh`) |

Sizes are decimal: `500MB` is 500,000,000 bytes, the base the tool prints in. Write `500MiB` for binary. Durations: `30m`, `48h`, `7d`, `1w`.

**JSON.** `--json` prints the whole `Report`: projects → worktrees → artifacts, each artifact with `bytes`, `growth_bytes`, `mtime_max`, `ecosystem`, `track`; `reconciliation`; `summary.by_type`; `notes` naming what the walk could and could not see.

```bash
slop-livin report ~/src --json --no-observe | jq '.summary.by_type'
slop-livin report ~/src --json --no-observe | jq '[.projects[].worktrees[].artifacts[] | select(.growth_bytes > 1e9) | {path, growth_bytes}]'
slop-livin report ~/src --json --no-observe | jq '.reconciliation'
```

<details>
<summary>Trimmed real output</summary>

```json
{
  "observed_at": 1789774367,
  "root": "/Users/me/src",
  "projects": [{
    "name": "slop-livin",
    "remote": "github.com/open-horizon-labs/slop-livin",
    "ecosystems": ["rs"],
    "worktrees": [{
      "path": "/Users/me/src/open-horizon-labs/slop-livin",
      "kind": "Main", "branch": "main", "idle_secs": 32,
      "signals": [{"name": "last_commit", "value": "last commit 1m"}, {"name": "dirty", "value": "clean"}],
      "artifacts": [{
        "kind": "BuildOutput", "path": "/Users/me/src/open-horizon-labs/slop-livin/target",
        "bytes": 16206716928, "growth_bytes": 10367176704, "mtime_max": 1789774334,
        "ecosystem": "rs", "track": "Ignored", "regrowth_count": 0
      }]
    }]
  }],
  "reconciliation": {"attributed": 44501635072, "unowned": 202854400, "walked_total": 44704489472,
                     "docker_attributed": 32240000, "docker_unowned": 15766421579},
  "summary": {"projects": 56, "worktrees": 89, "artifacts": 156,
              "by_type": {"rs": {"name": "Rust", "projects": 11, "artifacts": 3, "bytes": 20116369408, "growth_bytes": 10367176704}}},
  "notes": ["fsevents: mode=incremental reason=incremental changed_dirs=156"]
}
```

</details>

**Configuration** lives at `~/.local/share/slop-livin/config.toml` (`slop-livin config path`). `config show` prints the effective values; `config init` writes them out with their meaning:

```toml
since = "24h"                 # default growth window; --since overrides per call
retention_days = 30           # observation history kept before deltas are pruned
large_file_min_bytes = 1048576  # files at least this large get their own row under --dirs
observe_timeout_sec = 1800    # watchdog for one observe run
```

`SLOP_LIVIN_DIR` relocates the store. The TUI's filter, sort and keep-executables choice persist in `ui_state.json` next to it.

### Agent (MCP)

`slop-livin-mcp` speaks MCP over stdio. It is the primary operator surface: an agent asks what grew, proposes a plan, and executes it once a human has authorized it. It cannot authorize anything itself.

```bash
claude mcp add slop-livin ~/.local/bin/slop-livin-mcp        # Claude Code
```
```json
{ "mcpServers": { "slop-livin": { "command": "/Users/me/.local/bin/slop-livin-mcp" } } }
```

| Tool | Arguments | Answers |
|---|---|---|
| `report` | `root`, `since`, `project`, `view`, `filter`, `dirs` | the full report, or one `view` (`types`, `builds`, `deps`, `worktrees`, `docker`, `kinds`, `unowned`, `reconciliation`), narrowed by the same filter grammar as the CLI |
| `what_grew` | `root`, `since` | rows that grew in the window, sorted, with coverage and the history the store actually holds |
| `list_projects` | `root`, `since` | ranked projects: `owner/repo`, bytes, growth, checkout and worktree counts |
| `list_worktrees` | `root`, `since`, `filter` | branch, idle time, merge-complete with its terms, PR state |
| `docker_objects` | `root`, `project`, `unowned_only` | every image / build-cache entry / volume with created date, containers, shared layers |
| `propose` | `root`, `filter`, `paths`, `since` | a plan: every unit with bytes, growth, recovery contract, git status, signals and warnings |
| `execute` | `plan_id`, `keep_executables` | run an authorized plan: Trash, ledger, measured free space; refused units name the fact |
| `plans` · `grants` | | read-only listings |

Every answer carries `observed_at`, the `since` it actually used, and `history` when the asked window is longer than the store holds. Facts, with their terms, never verdicts: a worktree is `merge_complete: unknown (merged=unknown, clean=yes, unpushed=0, tip_reachable=unknown)`, not "stale".

**Recipes** (the agent's calls; `arguments` shown):

```jsonc
// "What ate my disk this week?"
{"name": "what_grew", "arguments": {"root": "/Users/me/src", "since": "7d"}}
// → grown: [...], coverage: {history: {asked_window_secs: 604800, effective_window_secs: 29308,
//     note: "asked for 604800s of growth but the store holds 29308s of observations; …"}}

// "Which Rust projects have build output over a gig?"
{"name": "report", "arguments": {"root": "/Users/me/src", "view": "builds", "filter": "type:rust size > 1GB"}}

// "Per ecosystem, where are the bytes?"
{"name": "report", "arguments": {"root": "/Users/me/src", "view": "types"}}
// → {"rs": {"name": "Rust", "projects": 11, "artifacts": 3, "bytes": 20116369408, "growth_bytes": 10367176704}, …}

// "Which branches are merged and idle for two days?"
{"name": "list_worktrees", "arguments": {"root": "/Users/me/src", "filter": "merge-complete idle > 48h"}}

// "Propose deleting Rust build output nobody wrote to in a month."
{"name": "propose", "arguments": {"root": "/Users/me/src", "filter": "kind:BuildOutput type:rust age > 30d"}}
// → {"state": "awaiting-authorization", "id": "cb86f0ba-…", "planned_bytes": 19754430464,
//    "units": [{"path": "…/slop-livin/target", "bytes": 16329043968, "recovery": "local_rebuild",
//               "track": "Ignored", "signals": ["last commit 1m", "clean", "0 unpushed", …], "warnings": []}],
//    "next_step": "a human authorizes with `slop-livin approve cb86f0ba-…` (this plan) or a standing
//                  `slop-livin grant add ...`; then call execute with plan_id. This tool cannot authorize."}

// After the human ran `slop-livin approve cb86f0ba-…`:
{"name": "execute", "arguments": {"plan_id": "cb86f0ba-…", "keep_executables": true}}
// → {"state": "executed", "outcomes": [{"path": "…/target", "status": "completed",
//    "recovery_location": "~/.Trash/target-slop-livin-1789774700", "preserved": ["…/slop-livin/bin/release/slop-livin"]}],
//    "trashed_bytes": 16329043968, "freed_measured": 0}   // Trash keeps the bytes until emptied; it says so
```

Authorization is a human at the keyboard:

```bash
slop-livin approve <plan-id>                                             # this plan, once; prints each unit's warnings first
slop-livin grant add 'kind:BuildOutput idle > 30d' --budget 5GB --expires 7d   # standing, bounded
```

Plans expire after 30 minutes and are single-use. A grant is checked per unit at the sink against the disk as it is then, not as it was proposed. There is no MCP tool that writes a grant. That is the design, not an omission.

## What it knows

**Projects, not paths.** A project is its git object store, unified across clones by remote, so two checkouts of `roon-knob` in different directories are one project with two checkouts and their linked worktrees. Rows are named `owner/repo`.

**Project types from markers, artifacts from types.** `Cargo.toml` makes a checkout Rust, `package.json` Node, `pyproject.toml` Python, and so on for twenty ecosystems; the same table says which directories each one generates. Unambiguous names (`node_modules`, `target`, `.venv`, `.stack-work`) are artifacts anywhere. Names several tools use (`build`, `dist`, `vendor`, `bin`, `obj`, `out`, `*.egg-info`) count only next to a marker of an ecosystem that generates them, so a repo's hand-written `build/` directory is source, and `.NET`'s `bin/` is build output. This is clean-dev-dirs' per-language detection, kept as a table so the CLI, the TUI, the MCP server and the filter all read one truth. A checkout with no remote is named from its manifest (`[package] name`, `"name"`, `module`, `<artifactId>`, …) instead of its directory.

<details>
<summary>Ecosystems: markers, artifacts, badge</summary>

| Badge | Ecosystem | Detected by | Artifacts it generates |
|---|---|---|---|
| 🦀 | Rust | `Cargo.toml` | `target/` |
| ⬢ | Node.js | `package.json` | `node_modules/`, `dist/`, `build/`, `out/`, `.next/`, `.nuxt/`, `.svelte-kit/`, `.output/`, `.turbo/`, `.parcel-cache/`, `.angular/`, `.expo/`, `.metro/`, `coverage/` |
| 🦕 | Deno | `deno.json`, `deno.jsonc` | `vendor/`, `node_modules/` |
| 🐍 | Python | `pyproject.toml`, `requirements.txt`, `setup.py`, `setup.cfg`, `Pipfile`, `poetry.lock`, `environment.yml` | `__pycache__/`, `.pytest_cache/`, `.mypy_cache/`, `.ruff_cache/`, `venv/`, `.venv/`, `build/`, `dist/`, `.eggs/`, `*.egg-info/`, `.tox/`, `.nox/`, `__pypackages__/`, `.pixi/` |
| 🐹 | Go | `go.mod` | `vendor/` |
| ☕ | JVM | `pom.xml`, `build.gradle(.kts)`, `settings.gradle` | `target/`, `build/`, `.gradle/` |
| 🔺 | Scala | `build.sbt` | `target/` |
| 🔧 | C/C++ | `CMakeLists.txt`, `meson.build`, `Makefile` | `build/`, `cmake-build-*/` |
| 🐦 | Swift | `Package.swift`, `*.xcodeproj`, `*.xcworkspace`, `Podfile` | `.build/`, `.swiftpm/`, `DerivedData/`, `Pods/` |
| 🟣 | .NET | `*.csproj`, `*.fsproj`, `*.vbproj`, `*.sln` | `bin/`, `obj/` |
| 💎 | Ruby | `Gemfile` | `.bundle/`, `vendor/` |
| 💧 | Elixir | `mix.exs` | `_build/`, `deps/`, `.elixir-tools/`, `.elixir_ls/`, `.lexical/` |
| 🐘 | PHP | `composer.json` | `vendor/` |
| λ | Haskell | `stack.yaml`, `*.cabal`, `cabal.project`, `package.yaml` | `.stack-work/`, `dist-newstyle/` |
| 🎯 | Dart/Flutter | `pubspec.yaml` | `.dart_tool/`, `build/` |
| ⚡ | Zig | `build.zig` | `zig-cache/`, `.zig-cache/`, `zig-out/` |
| 🌍 | Terraform | `*.tf` | `.terraform/` |
| 🐳 | Docker | `Dockerfile`, `compose.y(a)ml`, `docker-compose.y(a)ml` | (objects come from the daemon) |
| 🎲 | Unity | `ProjectSettings/` | `Library/`, `Temp/`, `Obj/`, `Logs/`, `MemoryCaptures/`, `Build/`, `Builds/` |
| 🎮 | Unreal | `*.uproject` | `Binaries/`, `Intermediate/`, `Saved/`, `DerivedDataCache/`, `Build/` |

Plus ESP-IDF (📟, from `sdkconfig`), Godot (🤖), Jekyll (📄), Elm (🌳), Erlang (☎️), OCaml (🐫), Clojure (🔮) and Nim (👑), and names that are artifacts anywhere: `.cache`, `.git`, `.xwin-cache`, `.ipynb_checkpoints`, `.terraform`.

**Where the names come from.** A directory that holds a `CACHEDIR.TAG` with the [Cache Directory Tagging Specification](https://bford.info/cachedir/) signature in its first 43 bytes is a cache with no further argument: the tool that made it says so. Everything else is a name plus a marker, and the names are harvested from [github/gitignore](https://github.com/github/gitignore)'s 163 templates and [linguist](https://github.com/github/linguist)'s vendored-paths list, both vendored under `vendor/`. `cargo run -p slop-livin-harvest` reports what upstream lists that the table lacks; it never writes the table, because a `.gitignore` entry proves a path is generated, not that deleting it is safe (Python's template lists `var/`, `instance/` and `lib/`). Its `--challenge <root>` mode uses a real tree only to contradict a candidate, by finding it holding git-tracked content. The table is `crates/core/src/ecosystem.rs`; a new ecosystem is one entry.

</details>

**Artifacts as units.** `node_modules`, `target`, `build`, `dist`, `.venv`, `.cache`, `.git` and friends are folded: sized as one unit, never descended, attributed to the nearest containing checkout. Deleting one is one action. Everything that isn't an artifact is the worktree's `source`, which expands into its own directories.

**Git status on every row.** `tracked`, `ignored`, or `untracked` — from git's own exclude rules via gitoxide, verified against `git check-ignore`. Untracked bytes are the ones no clone brings back.

**Activity as facts.** Last commit age, dirty, unpushed count, locked, idle time, computed in-process at walk time. With `gh` available, every GitHub-remote worktree is enriched with its PR (number, state, review) and whether the branch is merged into the default branch, one GraphQL call per repository, cached six hours. The composite `merge-complete` is always shown with its terms; the tool never says "safe" or "stale".

**Docker as part of the same answer.** Images, build cache and volumes are joined to projects only on explicit evidence: a compose label, a compose file's `name:` inside a worktree, or `org.opencontainers.image.source` matching a remote. Everything else is listed as unowned with the reason. Name similarity never attributes. Docker bytes are reported next to, not inside, the filesystem total.

**Honest totals.** `attributed + unowned = walked`, to the byte, on every report. `--verify-du` runs `du -skPx` as an independent oracle. Coverage names permission-denied directories rather than hiding them.

## How it works

**The pipeline is consumers on an event bus.** Walk, project grouping, git signals, GitHub, Docker, ecosystems, the growth store, tracking, history and assembly are each one `Consumer` in `crates/core/src/consumers/`, woken by the events they subscribe to and emitting facts; the bus (`crates/core/src/bus/`, tokio) is the only coupling. A new fact source is one file plus one line in `EventBus::with_builtins()`. Every technical constraint of the design is a guardrail in `.oh/guardrails/` with an AST audit in `crates/source-audit` (`cargo run -p slop-livin-source-audit -- --list`); the build fails when one is broken. ADR: `docs/ADRs/001-event-bus-report-pipeline.md`.


**Column store with reverse deltas.** Each observation writes the current state to Parquet (zstd) plus an append-only delta holding the *previous* values of rows that changed. Growth over any window is a lookup, not a rescan; a no-change observation appends nothing; deltas compact and age out on a configurable retention window. On the test machine the store for 56 projects, 89 worktrees and 25 k directory rollups is a few hundred KiB.

**Directory folding keeps it small.** There are no per-file rows. Artifact trees are one row each; Source trees get per-directory rollups (with `mod_time_min` as int32 minutes) and rows only for files over 1 MiB.

**FSEvents-driven incremental walks.** The stream id is stored with each observation. The next observation replays events since that id, re-sizes only the changed subtrees, and carries every other row forward; hardlinks are accounted per row so a re-size can never double-count. It refuses and falls back to a full walk on any doubt (no stored id, id from the future, history exhausted, too many changes, classification rules changed), and says which in `notes`. Full walk of ~45 GB / 89 worktrees: ~7 s; incremental: 2–9 s depending on how much moved. The result matches a forced full walk to the byte.

**Parallel walk.** Directory reads run on a bounded pool, one `lstat` per entry, staying on one device, never following symlinks.

**One report, three surfaces.** The TUI, CLI and MCP render the same `Report`. Nothing is computed only for the screen.

**Statically checked constraints.** Nineteen `syn`-based audits (`cargo run -p slop-livin-source-audit -- --list`) fail the build when a guardrail is broken: a walker that follows a symlink or folds a directory that is not an artifact, a Parquet writer without zstd, an observation that walks before asking FSEvents, a consumer that names another consumer, verdict vocabulary in output, a second byte formatter. `scripts/audit-mutants.sh` applies seven wrong-but-plausible walkers and shows each fails the audit that owns the rule; every mutation is anchored on exact text and asserts it applied, so the check cannot quietly stop testing.

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

Everything above was measured on one machine, one developer, over one day: 56 projects, 89 worktrees, ~45 GB under `~/src`, 270 Docker objects. Treat the timings as one data point, not a benchmark.

## Design notes

`PRODUCT.md` and `DESIGN.md` carry the product record and the TUI's design contract. `.oh/sessions/` holds the reasoning: aim, problem space, the salvage of the first attempt, and the plan. The short version: the unit of decision is the project, activity is the deciding attribute, and the tool states facts while the human decides.
