# swamp

Disk growth, by project.

Swamp keeps a history of disk usage across your Git projects. It groups checkouts, linked worktrees, build output, dependencies, and Docker objects so you can trace a change in disk space back to the project and artifact that grew. Git status and pull-request information give you context before you remove anything.

It is built for developers working across several projects and branches, including people running coding agents that build and install dependencies throughout the day.

## Start with what changed

A large directory may have been large for months. When your disk fills up this afternoon, the useful question is what grew since this morning.

Swamp records observations and compares sizes over a chosen window. Open a project to see which checkout or worktree changed, then inspect its build output, dependencies, caches, and remaining files. Separate clones with the same normalized Git remote appear under one project.

For example, a project might contain this mix of storage. This is an illustration of the model, not captured output:

```text
acme/api
├─ main checkout
│  ├─ target/          build output
│  ├─ .git/            repository data
│  └─ remaining files tracked, ignored, or untracked
├─ feature worktree
│  ├─ target/          its own build output and growth
│  └─ remaining files
└─ Docker objects     joined through labels or a matching source remote
```

The same model supports questions at different levels:

- Which project grew in the last day?
- Is the growth in a worktree's build output, its dependencies, or its other files?
- Which worktrees have merged branches, no unpushed commits, and no recent activity?
- Did an artifact reappear after being removed?

History begins with the first observation. Swamp records sizes and metadata; it does not back up file contents or identify the process that wrote them.

## Platforms

macOS on Apple silicon (`aarch64-apple-darwin`) and Linux on x86_64
(`x86_64-unknown-linux-gnu`, validated on Ubuntu 24.04). CI builds and runs the
whole test suite natively on both on every push.

On Linux, a release archive is published alongside the macOS one (glibc,
generic x86-64, built on Ubuntu 24.04). Linux has no persisted change history,
so an observation there walks fully unless a live watch -- the TUI, or the
opt-in `swamp collect` -- has been running since the last one; scheduling uses
`systemd --user`, and Trash is the freedesktop one your file manager shows. The
[platform guide](docs/platform.md) has the whole table and the reasons.

## Install

On Apple silicon macOS, install with [Homebrew](https://brew.sh):

```bash
brew install open-horizon-labs/tap/swamp
swamp --version
```

Update with `brew upgrade swamp`. See the [installation guide](docs/usage.md#installing-a-release) if you previously installed manually.

On Linux x86_64, install the release archive after checking its checksum:

```bash
curl -LO https://github.com/open-horizon-labs/swamp/releases/latest/download/swamp-x86_64-unknown-linux-gnu.tar.gz
curl -LO https://github.com/open-horizon-labs/swamp/releases/latest/download/swamp-x86_64-unknown-linux-gnu.tar.gz.sha256
sha256sum -c swamp-x86_64-unknown-linux-gnu.tar.gz.sha256
tar -xzf swamp-x86_64-unknown-linux-gnu.tar.gz
install -m 755 swamp-x86_64-unknown-linux-gnu/swamp ~/.local/bin/
```

### Build from source

Build the current `swamp` version from source with a recent stable Rust toolchain:

```bash
git clone https://github.com/open-horizon-labs/swamp
cd swamp
cargo build --release --locked -p swamp
mkdir -p ~/.local/bin
install -m 755 target/release/swamp ~/.local/bin/
~/.local/bin/swamp --version
```

## Use it

```bash
swamp ui ~/src
swamp report ~/src --since 24h
swamp report ~/src --project api
```

In the terminal UI, use the arrow keys to navigate and open a project. `/` opens the filter form; `0` clears the filter. The initial filter is `growth > 100MB in 7d`, so clear it if you want to see projects that have not grown. Filter and sort choices are saved between sessions.

To collect history while the UI is closed:

```bash
swamp schedule --every 15m ~/src
```

This installs a per-user LaunchAgent (macOS) or `systemd --user` timer (Linux; add `--collector` to keep a live change list between runs) that observes the root and refreshes GitHub information through `gh` when available. It performs no cleanup. `swamp schedule` shows its status; `swamp schedule --off` removes it.

Every root above is explicit. Omit it and `report`/`observe`/`ui`/`schedule` resolve swamp's **effective scope** instead: built-in roots (`~/src`, `~/Library/Developer`, `~/Library/Caches`), plus detected developer-tool locations (Cargo, rustup, Homebrew, and more), plus anything you add or exclude in `config.toml`. Run `swamp scope` to see exactly what's in scope, why, and what's missing or excluded -- see [Scope and coverage](docs/usage.md#scope-and-coverage).

## Decide with context

Swamp shows artifact types, Git tracking status, dirty files, unpushed commits, worktree activity, and cached GitHub PR and merge information. An ignored file may hold private data. A tracked file may have uncommitted edits. Neither label establishes that a copy exists elsewhere.

Space marks rows in the UI. Backspace opens a confirmation for the selected row or marked set; Enter confirms. Read the paths and warnings: whole checkouts and linked worktrees can also be selected. Project-level actions expand into artifact rows, with a checkout fallback when no actionable artifacts exist. Bulk marking with `A` does not take that checkout fallback.

Filesystem removals move paths to Trash. Docker images and volumes are removed through Docker and have no Trash recovery; swamp does not make a backup. Build-cache entries are reported but cannot be removed individually through swamp. Moving files to Trash does not itself reclaim their disk space.

The CLI also supports proposals, human approval, and execution. See [cleanup and recovery](docs/usage.md#cleanup-and-recovery) before using it.

## Use it from an agent

There is no separate server process. `swamp report --json`, `--view <name> --json`, `propose --json`, and `execute --json` print one bounded JSON document to stdout with diagnostics on stderr -- safe for an agent to call directly and parse:

```bash
swamp report ~/src --view grown --json --since 24h
swamp propose ~/src --filter 'kind:BuildOutput idle > 30d' --json
```

Install the skill at `skills/swamp/` into your agent client's skills directory (copy or symlink it; see [installing the skill](docs/usage.md#agent-interface)) so the agent knows the exact commands, the JSON schema, and the authorization rule: an agent may propose a plan, but grant creation and plan approval (`swamp approve`, `swamp grant add`) are reserved for a human's explicit instruction. That rule is followed, not enforced by any wall between "agent" and "human" processes -- a shell-capable agent could type the same command. The real safety boundary is in swamp itself: every execution re-derives its units against the live filesystem, grants are scoped/budgeted/expiring, and every action is ledgered. See [the trust model](skills/swamp/references/trust-model.md).

## How updates stay small

After the initial walk, swamp asks the platform what changed and reuses the stored measurements elsewhere. It retains directory detail inside grouped artifacts so a small change can often be measured without walking the whole artifact again. Hardlinks and incomplete event history require broader walks.

On macOS that question is answered by FSEvents, which replays a log the kernel kept while swamp was not running. Linux has no equivalent: inotify reports only what happens while a watch is open, so an observation there walks fully and reports `reason=no_persisted_change_history` rather than treating an unwatched period as a quiet one. [#81](https://github.com/open-horizon-labs/swamp/issues/81) adds a live watcher, which narrows that gap for a running swamp and does not close it. See [Platforms](docs/platform.md).

The history store uses zstd-compressed Parquet, directory summaries, selected large-file rows, and reverse deltas containing previous values. These choices reduce repeated traversal and history storage. Actual work depends on the changed directories, hardlinks, and enrichment caches; the repository does not establish a general latency or storage-size guarantee.

The [architecture guide](docs/architecture.md) explains observation, history, enrichment, the project model, and extension points.

## Documentation

- [Usage](docs/usage.md): installation, keys, commands, filters, configuration, the agent interface, and recovery.
- [Platforms](docs/platform.md): supported targets, what each platform can and cannot do, where swamp keeps its files on each, and the library reuse decisions behind that.
- [Architecture](docs/architecture.md): data flow, incremental updates, storage, and implementation limits.
- [Contributing](CONTRIBUTING.md): code map, checks, and documentation maintenance.
- [Changelog](CHANGELOG.md): behavior introduced in each release.
- [Product](PRODUCT.md) and [terminal design](DESIGN.md): the current product and UI contracts.
