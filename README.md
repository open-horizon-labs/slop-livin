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

## Install

On Apple silicon macOS, install with [Homebrew](https://brew.sh):

```bash
brew install open-horizon-labs/tap/swamp
swamp --version
```

Update with `brew upgrade swamp`. See the [installation guide](docs/usage.md#installing-a-release) if you previously installed manually.

### Build from source

Build the current `swamp` version from source with a recent stable Rust toolchain:

```bash
git clone https://github.com/open-horizon-labs/swamp
cd swamp
cargo build --release --locked -p swamp -p swamp-mcp
mkdir -p ~/.local/bin
install -m 755 target/release/swamp target/release/swamp-mcp ~/.local/bin/
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

This installs a per-user LaunchAgent that observes the root and refreshes GitHub information through `gh` when available. It performs no cleanup. `swamp schedule` shows its status; `swamp schedule --off` removes it.

## Decide with context

Swamp shows artifact types, Git tracking status, dirty files, unpushed commits, worktree activity, and cached GitHub PR and merge information. An ignored file may hold private data. A tracked file may have uncommitted edits. Neither label establishes that a copy exists elsewhere.

Space marks rows in the UI. Backspace opens a confirmation for the selected row or marked set; Enter confirms. Read the paths and warnings: whole checkouts and linked worktrees can also be selected. Project-level actions expand into artifact rows, with a checkout fallback when no actionable artifacts exist. Bulk marking with `A` does not take that checkout fallback.

Filesystem removals move paths to Trash. Docker images and volumes are removed through Docker and have no Trash recovery; swamp does not make a backup. Build-cache entries are reported but cannot be removed individually through swamp. Moving files to Trash does not itself reclaim their disk space.

The CLI also supports proposals, human approval, and execution. See [cleanup and recovery](docs/usage.md#cleanup-and-recovery) before using it.

## Use it from an agent

`swamp-mcp` exposes the report over stdio, with tools for growth, projects, worktrees, Docker objects, and proposed actions. Configure an MCP client with the absolute path to the binary:

```json
{
  "mcpServers": {
    "swamp": {
      "command": "/Users/you/.local/bin/swamp-mcp"
    }
  }
}
```

An agent can propose a plan and execute an approved one. Grant creation is available through the CLI and TUI confirmation, with no MCP grant-writing tool. This is an interface boundary, not isolation from an agent that also has unrestricted shell access. The [MCP reference](docs/usage.md#agent-interface) lists the tools and authorization flow.

## How updates stay small

After the initial walk, swamp uses macOS FSEvents to find changed directories and reuses the stored measurements elsewhere. It retains directory detail inside grouped artifacts so a small change can often be measured without walking the whole artifact again. Hardlinks and incomplete event history require broader walks.

The history store uses zstd-compressed Parquet, directory summaries, selected large-file rows, and reverse deltas containing previous values. These choices reduce repeated traversal and history storage. Actual work depends on the changed directories, hardlinks, and enrichment caches; the repository does not establish a general latency or storage-size guarantee.

The [architecture guide](docs/architecture.md) explains observation, history, enrichment, the project model, and extension points.

## Documentation

- [Usage](docs/usage.md): installation, keys, commands, filters, configuration, MCP, and recovery.
- [Architecture](docs/architecture.md): data flow, incremental updates, storage, and implementation limits.
- [Contributing](CONTRIBUTING.md): code map, checks, and documentation maintenance.
- [Changelog](CHANGELOG.md): behavior introduced in each release.
- [Product](PRODUCT.md) and [terminal design](DESIGN.md): the current product and UI contracts.
