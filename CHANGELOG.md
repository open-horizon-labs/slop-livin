# Changelog

Release notes live here, one section per tag. The release workflow refuses a tag without one.

## v0.3.0

Watching instead of asking, and an incremental observation that is actually incremental.

**Live FSEvents**

- The TUI opens a live FSEvents stream on the root for its lifetime. Every change under it, including a Trash move the TUI itself just made, arrives as an event; after 400 ms of quiet the tool observes exactly those directories. Nothing asks for a refresh, and two deletes in a row cannot race each other. The header says `observed just now · live`.
- A live plan skips the replay-lag floor that a log replay needs: the event *is* the change, not a query that might predate it.

**Store depth inside folded artifacts**

- A folded artifact is still one row in the report and one unit of action, but the store now also holds per-directory rows for its interior. A change deep inside re-lists that one directory and re-aggregates, instead of re-stating every file under the unit. A changed Source directory is likewise re-listed in place rather than sending its whole worktree back to the walker.
- Summing directory rows is only correct when no inode appears twice in the unit. Cargo's `target/` hardlinks nearly every artifact: summing its rows overcounted a 16 GB tree by 4.4 GB against a full walk. Every artifact row now records whether its unit holds hardlinked files, measured by the walk and persisted, and the interior path runs only when it does not.
- Growth annotation builds one history index per observation instead of re-reading the store per artifact, and the three stores skip rewriting their current file when no row changed.
- Git signals are recomputed only for re-walked worktrees and aged for the rest; Docker facts are cached for five minutes; discovery at a changed directory looks one level deep, not the whole tree.

One observation on `~/src` (55 projects, 44 GB):

| Case | v0.2.0 | v0.3.0 |
|---|---|---|
| A source file was touched | 7.5 s | 75 ms |
| A change deep inside a 16 GB `target/` | 7.5 s | 2.4 s |
| Nothing changed | 7.5 s | 86 ms |

**Where artifact names come from now**

- `CACHEDIR.TAG` is honoured: a directory holding a regular `CACHEDIR.TAG` whose first 43 bytes are the [Cache Directory Tagging Specification](https://bford.info/cachedir/) signature is a cache, whatever its name and with no marker gate, because the tool that created it is the one making the claim. Cargo writes one into `target/`, pytest into `.pytest_cache/`, uv into `.venv/`. A file that merely mentions the string, or a symlink standing in for the tag, does not qualify.
- [github/gitignore](https://github.com/github/gitignore) (163 templates) and [linguist](https://github.com/github/linguist)'s vendored-paths list are vendored under `vendor/`, and `cargo run -p slop-livin-harvest` reports what they list that our table lacks. It never writes the table: the Python template alone lists `var/`, `instance/`, `lib/`, `mnesia/` and `rabbitmq/`, which hold authored or live state. Its `--challenge <root>` mode uses a tree only to *contradict* a candidate, by finding it holding git-tracked content; that is how `artifacts`, `generated`, `inc`, `packages`, `settings` and `Screenshots` were kept out. Sizes rank nothing.
- Curated from that report, marker-gated, each carrying its provenance: around 70 names across Python, Node, .NET, C/C++, JVM, Ruby, Dart, Deno, Haskell, Swift, Unity and Unreal, plus seven ecosystems we did not model at all — Godot, Jekyll, Elm, Erlang, OCaml, Clojure and Nim.

**Artifacts the table was missing**

- ESP-IDF is its own ecosystem, identified by `sdkconfig` / `idf_component.yml` / `partitions.csv` rather than a `CMakeLists.txt` it need not have. Its `build/`, per-variant `build-<target>/` and vendored `managed_components/` are artifacts.
- CMake projects also build into `build-<variant>/`; marker patterns now match a prefix as well as a suffix.
- Python is recognised from `requirements*`, so a repo whose only marker was `requirements-dev.txt` no longer keeps its `build/` in the Source total.
- Measured on one developer's tree: one ESP-IDF repo's Source dropped from 3.5 GB to 244 MB, with 3.2 GB moving into deps and build rows where it can be acted on. A sweep of every remaining Source directory over 50 MB found only genuine data (generated datasets, experiment artifacts, KiCad files, photos) and two small ignored temp directories.

**Charts that say something**

- Growth is red and shrink is green: in a disk tool, arriving bytes are the bad news.
- The signed number sits flush against its bar, no brackets, one diffstat token.
- Sparklines plot *when* bytes moved, one bar per bucket of change, rather than redrawing the tree's size as a solid block. A bucket where nothing moved is blank; one before the row was first observed is a dim `·`.
- The selected row is a dark background, not reverse video, so the growth colours stay readable.
- Ecosystem and fact badges trail the project name instead of prefixing it, so names stay left-aligned.

**Fixes**

- A background observation keeps the cursor on its row instead of jumping to the top.
- The TUI's startup observation produces per-directory rows, so a `source` row expands into its directories instead of reading `▸ 0 more`.
- Nested walks no longer reset the shared progress counters, which made the header read past the total and stick at 99%. The percentage is an estimate against the last observation and is now shown only while it means something.
- `slop-livin --version`, asserted against the tag by the release smoke test.

**Docs**

- The README was rewritten from real output: current frames, release-tarball install, a quick start, CLI recipes for every view, the filter grammar as a table, a trimmed real JSON report, the config file, MCP setup with request and response recipes, and a table of all twenty ecosystems with their markers and the artifacts each generates.

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
- Every technical constraint of the design (FSEvents before any walk, Parquet + zstd, reverse deltas, scheduled refresh, folding only for artifacts, symlinks never followed, incremental re-walks, the parallel pool, int32-minute mtimes, facts not verdicts, human-only authorization, pluggable consumers, one byte formatter) is a guardrail under `.oh/guardrails/` and a named AST audit in `crates/source-audit`; `scripts/check.sh` fails when one is broken. `scripts/audit-mutants.sh` proves the walker audits bite: seven wrong-but-plausible walkers (following a link, dropping the symlink guard, folding under a made-up kind, …) each fail the audit that owns the rule.

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
