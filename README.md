# slop-livin

Find out what grew on your disk, by project, and delete it from where you're standing.

`slop-livin` is for developers who run many coding agents at once. Every agent builds, installs, and caches, and a week later the disk is full and nobody remembers which project did it. `du` tells you what is big. `slop-livin` tells you what **grew**, attributes it to a git project → checkout/worktree → artifact, and lets you (or your agent) act on it with the facts in front of you.

```
~/src · observed just now · since 24h · 46 projects · 39.6GB attributed · 202.9MB unowned · docker 11.1GB unowned
view: projects · filter: none
open-horizon-labs/slop-livin  🦀🔨                      20.4GB    +14.6GB             │█████████████▏
obsidian-am  ⬢                                         81.2MB         0B             │
hiphi-repos/hiphi  ⎇1                                  72.6MB         0B             │
…
open-horizon-labs/agent-surface  ⬢🔨                     4.4GB     -1.9GB  ██████████▌│
open-horizon-labs/governed-compositional-etl  🐍🔨     972.2MB     -2.1GB  ██████████▊│
hiphi-repos/roon-knob  🔨                               5.2GB     -3.2GB ███████████▍│
```

Each row is a `git diff --stat` line for a project: size, then the signed change over the window, then its bar. The bar diverges around a dim axis — bytes that arrived grow right in red, bytes that left grow left in green — so direction is geometry rather than colour alone, and its length is logarithmic, so a 100 MB change and a 3 MB change do not look alike. What arrived sorts first and what left sorts last, because the question is what grew. Enter opens the project:

```
view: tree of agent-surface  (Esc back) · filter: none
└─ ▾ main ~/src/open-horizon-labs/agent-surface                      4.4GB     -1.9GB  ██████████▌│  dirty · 0 unpushed
   ├─ untracked .                                                   39.2MB     +1.1MB             │█████████▎
   ├─ artifacts firmware/agent_surface_stackchan (x2)                1.3GB         0B             │
   ├─ artifacts firmware/agent_surface_atom (x2)                     1.3GB         0B             │
   ├─ git .git                                                     302.1MB         0B             │
   ├─ build emulator/build                                          73.9MB         0B             │
   ├─ deps node_modules                                             45.3MB         0B             │
   └─ source .                                                      23.8MB         0B             │
```

That `untracked` line is the point. It is 39.2 MB, it is growing, it is in no index and no `.gitignore` covers it, so nothing would bring it back. It is listed apart from `source`, which is the 23.8 MB git actually tracks, and apart from `ignored`, which is what a gitignore rule matches. One row called "source" would have hidden all three in a single number. The tool states which is which before you press Backspace; it does not decide for you.

macOS only for now (FSEvents, LaunchAgent, Trash).

## Install

Apple silicon macOS. Each release ships a tarball and its checksum.

```bash
v=0.4.0
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
| `→` `←` | in / out: open or expand · collapse or go back |
| `Enter` | open a project (in the projects view); confirm a delete |
| `Space` | mark / unmark the row. On a project row, that is every artifact it holds |
| `A` | mark every row in this view the tool can act on, for one confirm |
| `⌫` | delete what is under the cursor (or the marked rows), after one confirm |
| `k` | keep executables: before trashing `target/`, copy `release/`/`debug/` binaries to `bin/` (Python: `dist/*.whl`, `build/**/*.so`) |
| `/` | filter form: growth, window, kind, project, idle, merge-complete, PR, type, size, age |
| `:` | filter as text, with Tab completion |
| `0` | clear the filter |
| `v` `1`–`8` | views: projects · tree · builds · deps · docker · kinds · unowned · types |
| `g` `s` `n` `t` `a` | sort by growth / size / name / type / age; `r` reverses |
| `Esc` | back to projects |
| `?` | help |

**Columns.** bytes · the signed change over the window · a diverging bar around a dim axis, shrink left in green and growth right in red, logarithmic so four orders of magnitude are all legible, with a change under 1 MB drawn as one tick against the axis and its number dimmed · facts (`last commit 1h · dirty · 26 unpushed`). Sorting by growth ranks what arrived first and what left last, because the question is what grew.

**Row kinds** under a worktree are what the bytes are, not where they sit: `build`, `deps`, `cache`, `artifacts` and `git` for folded directories, `docker-image` / `docker-volume` for objects joined to the project, then the three that split the rest of the checkout by what git says about it — `source` for what git tracks, `ignored` for what a gitignore rule matches, `untracked` for what is in no version control at all. The last two are aggregates over many scattered paths, so they are reported, never deleted as a unit.

**Badges** after a project's name are its ecosystems, from the markers at the checkout root: 🦀 Rust · ⬢ Node · 🦕 Deno · 🐍 Python · 🐹 Go · ☕ JVM · 🔺 Scala · 🔧 C/C++ · 🐦 Swift · 🟣 .NET · 💎 Ruby · 💧 Elixir · 🐘 PHP · λ Haskell · 🎯 Dart · ⚡ Zig · 🌍 Terraform · 🐳 Docker · 🎲 Unity · 🎮 Unreal, plus 🔨 when it holds build output and ⎇N for N linked worktrees. A checkout can be several at once.

**Deleting.** Backspace asks once, and the question states what the tool knows:

```
delete node_modules ⚠ dirty · 26 unpushed (331.0MB) → Trash?  Enter yes · Esc no · k keep executables
```

You do not have to open a project to act on it. Space or Backspace on a projects row takes every artifact that project holds — dependency trees, build output, caches and its Docker objects — behind one confirm. The checkout, its `.git` and its source tree are not in that set; removing those stays a deliberate act one level in, on the row that names the worktree and carries its warnings. A project with nothing rebuildable in it offers the checkout itself, because that is the only thing it has.

Anything that cannot be restored is named on the confirm line, not just counted:

```
delete node_modules, target, .cache +2 more (2.1GB) → 1.6GB to Trash, 501.0MB removed permanently (docker, no Trash) · gone for good: app-data (docker volume), slop-app:latest (docker image)?  Enter yes · Esc no · k keep executables
```

Paths go to Trash. Docker objects do not: the daemon removes them and there is no copy anywhere, so they are listed by name. The row and everything under it leave the screen at once, the totals drop by what left, and an incremental re-observe runs in the background. The ledger (`~/.local/share/slop-livin/ledger.jsonl`) records what was removed, by whom, and which warnings were on screen.

Filter, sort and the keep-executables setting persist between sessions. The growth window can only be as long as the history the store holds; the header says `since 4h (asked 1w; history is 4h)` rather than pretending.

### Docker

Images, build cache and volumes are part of the same answer, and can be removed from the same keystroke. The docker view (`5` in the TUI, `--view docker` on the command line) lists every object, joined to a project or not. From the command line:

```
project        object                      kind            bytes   shared  facts (trimmed)
slop-fixture   slop-fixture-app:latest     docker-image   43.9MB        —  created 22:40 · no containers reference it
slop-fixture   slop-fixture-worker:latest  docker-image   18.7MB        —  created 22:40 · containers=slop-fixture-held (created)
slop-fixture   slop-fixture-app-data       docker-volume  33.5MB        —  created 22:40 · no containers reference it
               slop-fixture-orphan:latest  image          27.1MB   4.2MB  created 22:40 · no containers reference it
               sha256:0bb6d5b540a1…        image          14.5MB   4.2MB  created 23:07 · dangling
```

The first three are joined to a project on evidence; the last two are unowned, and the report says so rather than guessing an owner. `Space` marks, `⌫` asks once, and the daemon does the removal: `docker image rm` or `docker volume rm`. Four things are worth knowing before you press Enter.

| | |
|---|---|
| **Nothing reaches Trash** | A path can be dragged back out of Trash; a Docker object cannot. Removal is permanent and the confirm line names each object rather than only counting its bytes. |
| **The recovery contract is per kind** | An image is `pull_or_rebuild (permanent: no Trash)`. A volume is `irrecoverable (permanent: no Trash, no copy anywhere)` — its contents exist in exactly one place. Build cache is `local_rebuild (permanent: no Trash)`. |
| **Build cache is refused per entry** | Docker exposes no per-record removal for it, only `docker builder prune`, which acts on everything reclaimable at once. That is a different unit of action than a plan unit, so the tool reports build cache and refuses to pretend it can remove one entry. |
| **A refusal comes back in the daemon's own words** | An image a container still holds is refused by Docker, and the outcome says what Docker said, not a guess. |

Orphans are removable too. An image with no join evidence, a dangling layer or a volume no project claims appears under `unowned` (view `7`) with the reason it is unowned, and marks like anything else. Nothing is joined to a project on name similarity, so an orphan says "no project claims these bytes" rather than guessing at one.

From the command line the same objects go through propose → approve → execute:

```bash
slop-livin report ~/src --view docker                            # joined and unowned together
slop-livin propose ~/src --filter 'kind:DockerImage project:my-app'
slop-livin approve <plan-id>                                     # prints each unit's warnings and recovery contract
slop-livin execute <plan-id>                                     # refused units name the fact that refused them
```

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

**Docker as part of the same answer.** Images, build cache and volumes are joined to projects only on explicit evidence: a compose label, a compose file's `name:` inside a worktree, or `org.opencontainers.image.source` matching a remote. Everything else is listed as unowned with the reason. Name similarity never attributes. Docker bytes are reported next to, not inside, the filesystem total. Images and volumes are removable from the same keystroke as a directory, through the daemon rather than Trash, which is why their recovery contract is stated and each object is named on the confirm; build cache is reported and refused per entry, because Docker has no per-record removal for it.

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
