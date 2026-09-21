# Changelog

Release notes describe behavior at the named version. See the [README](README.md) and [usage reference](docs/usage.md) for current behavior. Timings below are historical observations from one developer's machine, not a benchmark suite.

## Unreleased

- **Modeled agent-tool storage (Claude Code) and shipped a supported
  cleanup path** (#91, #92, #100, #101). `swamp report --view agents`
  identifies sessions, caches, logs, checkpoints and protected
  configuration under an agent-coding tool's home directory (Claude
  Code's `~/.claude` or `$CLAUDE_CONFIG_DIR` this release), linking
  each session to a swamp project where its transcript declares a
  `cwd` -- never a basename guess. History reuses the exact same
  current+reverse-delta growth-store key family `external.rs`'s
  detector-resolved units already use (an `"agent:"`-prefixed category
  string, never a second store). A required 13-tool matrix
  (`crates/core/src/agents/matrix.rs`) names every major coding-agent
  tool with a sourced home-path note; only Claude Code has real
  identification code this release, the rest are explicitly `Planned`.
  New `swamp protect add/list/remove` for human keep intent (survives
  refresh, independent of the growth store) and `swamp propose-agents
  --path <unit-path>` for a real, actionable plan -- cache/log
  categories move to Trash as a whole directory; an individual session
  removal moves its exact member set (transcript, subagent dir,
  file-history, todos) together, with membership re-verified fresh at
  execution -- through the same `swamp approve`/`swamp execute` every
  other plan uses. Credentials, settings, skills, commands and
  automation definitions are protected by default and have no
  supported action; neither does a database-like (SQLite/WAL/SHM)
  filename, or an active session (an `lsof`-style occupancy check on
  the transcript). Identification never reads past a session
  transcript's first line, and never puts prompt/response/attachment/
  credential content into a report, plan, or the ledger. The TUI gained
  two new minimal, read-only views: `ViewKind::External` (`'9'`, the
  minimal design chunk B2 recorded but did not implement) and
  `ViewKind::Agents` (no dedicated digit; reached by cycling with `v`),
  plus a header scope-coverage clause (`2 roots (1 missing)`) for its
  own root, scoped down from the full multi-root vision (#50) to what
  is available without walking anything. See `docs/agent-storage.md`
  for the full contract and known gaps (the other 12 named tools, TUI
  mark/confirm for agent actions, `~/.claude.json` living outside the
  modeled home directory).
- **Identified Codex, Codex's desktop app, Oh My Pi and OpenCode
  storage, and wired agent-storage actions into the TUI** (#93, #94,
  #95, #101). Four new adapters bring `swamp report --view agents` (CLI
  and TUI) to five supported tools: Codex (`CODEX_HOME`, default
  `~/.codex`) identifies live/archived rollout sessions in their
  year/month/day date trees and six SQLite state stores
  (`state_5.sqlite`, `logs_2.sqlite`, `goals_1.sqlite`,
  `memories_1.sqlite`, `queue_1.sqlite`, `thread_history_1.sqlite`),
  each folded with its `-wal`/`-shm` sidecars into one protected,
  non-actionable unit; the Codex desktop app is modeled as its own row
  covering only its confirmed macOS log directory
  (`~/Library/Logs/com.openai.codex`), never extrapolating the CLI's
  schema onto it. Oh My Pi (`~/.omp/agent`, confirmed by the user as a
  fork of `badlogic/pi-mono`) identifies sessions with a fixed 256-byte
  title-slot header, verifies content markers before treating anything
  under `~/.omp` as its own format (an explicit "unknown format" unit
  otherwise, since other tools can plausibly use that path), and tracks
  its content-addressed `blobs/` store's per-session reference counts
  with a bounded, per-session body scan -- never offering blob removal
  in this release, since complete reference coverage is not
  established. OpenCode identifies both its older file-tree layout
  (`storage/session/<project>/<session>.json`, linked via
  `storage/project/<id>.json`'s declared `worktree` field -- no
  session-body read needed at all) and its newer SQLite-backed one
  (`opencode.db`), version-gated by which markers are present on disk,
  plus its git-backed `snapshot/<project>/` checkpoint store
  (identified, linked, never actionable -- removing it loses `/undo`
  history). Two new `AgentMemberKind` variants (`Database`,
  `SessionData`) and a shared `agents::resolve_declared_path` helper
  (factored out of Claude Code's original implementation) support all
  four adapters without duplicating the project-linkage git-walk logic.
  The TUI's Agents view is now markable: `Space`/`Backspace` mark a
  supported unit and open the confirm banner with its real consequences
  (loss warnings, linked project), `Enter` executes through the
  existing background-worker path (never blocking the event/render
  thread), and a protected/unsupported row's footer names
  `propose_agents`'s own refusal reason. See `docs/agent-storage.md`
  for the full per-tool detail and remaining scope boundaries (blob
  GC, snapshot removal, TUI bulk marking, unconfirmed env var names).
- **Made multi-root observation coverage-aware** (#42). `report`,
  `observe`, and `ui` with no explicit root now observe the whole
  configured scope coherently in one call, not just its first present
  root: every present root is walked, every root's own current+reverse-
  delta growth store is written (each root still keeps its own
  physical store; this is a coherent orchestration, not a merged
  store), and `report --json` gains a `scope_coverage` array naming
  each root's outcome (`complete`/`partial`/`excluded`/`missing`/
  `inaccessible`) with a reason whenever it is not simply `complete`. A
  root that loses read access (`chmod 000`, or a worktree inside an
  otherwise-readable root) is distinguished from one that is genuinely
  deleted: losing and regaining access never fabricates a deletion or a
  later regrowth, and a root dropped from scope (or newly excluded) is
  a coverage change, never a storage change. `EffectiveScope::pruned_subtrees`
  (#41, recorded but not previously consumed) is now wired into the
  walker, so an `exclude` entry inside a kept root is genuinely not
  measured. `swamp schedule --every` installed with no explicit roots
  no longer freezes a resolved root list into the LaunchAgent's argv:
  every scheduled fire re-resolves the configured scope, so a
  `config.toml` edit takes effect on the next run.
- **Modeled external/shared storage as first-class measured units**
  (#43): the Cargo registry, rustup toolchains, Homebrew, and future
  detector-resolved locations are measured independently of any
  project/worktree, with size/growth/regrowth history in the same
  current+reverse-delta growth store (a new key family, not a second
  store) and declared consumer associations (zero/one/many, counted
  once, never duplicating the unit or resetting its history). New
  `report --view external` (text and `--json`). External units are
  inspection-only: a plan can name one, but `execute` refuses every one
  of them unconditionally with "no supported selective action for
  `<category>`" -- registry/detector output never authorizes removal.
- **Removed `crates/mcp`/`swamp-mcp`.** The CLI's `--json` output is now the sole supported agent interface. `report --json` honors `--view`/`--project`/`--filter` (it previously ignored them and dumped the whole report); gains `--limit`/`--offset` with `total`/`truncated` envelope fields for bounded results; and gains two JSON-only views, `--view projects` and `--view grown`, covering the former `list_projects` and `what_grew` MCP tools. `propose --json` carries the same `state`/`next_step`/`observed_at` fields the MCP `propose` tool added. `plans --json` and the new `grant list --json` wrap their arrays with a `total` field.
- Added an installable agent skill at `skills/swamp/` (`SKILL.md` plus lazily loaded `references/*.md`): the inspect-first workflow, the full former-MCP-tool-to-CLI-command mapping and JSON schemas, the filter grammar, the propose/approve/execute/grant lifecycle, coverage/history semantics, and the real (transport-independent) authorization trust model.
- Rewrote the `human_only_authorization` source audit from "the MCP server never calls these functions" to a transport-independent check: authorization-minting functions are called only from the CLI's own approve/grant command handling or the TUI's confirmed-execution path, regardless of which binary a caller invokes.
- **Replaced the hand-rolled `config.toml` line parser with a real TOML parser** (#41). Every previously supported scalar key (`since`, `retention_days`, `large_file_min_bytes`, `observe_timeout_sec`) keeps its meaning and default; a config file that fails to parse, or whose `[scan]` table has a wrongly-typed field (e.g. `defaults = "yes"` instead of a bool), is now a visible, nonzero-exit error at every scope-resolving CLI entry point (`scope`, `report`, `observe`, `ui`, `schedule`, `config show`) rather than a silent fallback to defaults.
- **Added an effective-scope model and a new `swamp scope [--json]` command** (#41). `report`/`observe`/`ui`/`schedule` now resolve a root when none is given, instead of defaulting to the current directory: built-in default roots (`~/src`, `~/Library/Developer`, `~/Library/Caches` on macOS) plus enabled location-detector results, plus `config.toml`'s new `[scan]` table (`defaults`, `include`, `exclude`, `disabled_detectors`). Exclusions always win, including over an explicit command-line root; a detector disabled by `disabled_detectors` does not hide a path still reachable through another enabled root; a root nested inside another in-scope root is folded into its parent for measurement while the fold is retained as an inclusion reason. An effective scope that resolves to nothing at all is a visible, explicit error -- never a silent fallback to the current directory or home. `observe`/`schedule` walk every present root in the resolved scope; `report`/`ui` remain single-root for this change and use the scope's first present root (full multi-root `report`/`ui` support is #42/#50). `report`/`observe` persist the resolved scope and print a `coverage changed since last observation: ...` note on stderr when it differs from the last one -- a coverage change, never a byte-history delta or tombstone.
- **Added a source-aware location-detector registry** (`crates/core/src/locations/`, #44): a small, statically registered catalog (built-in default roots, Cargo home, rustup, Homebrew) that proposes developer-storage locations with stable IDs, storage categories, and resolution provenance (built-in convention / env var / config field / bounded read-only tool query), without ever authorizing measurement or removal. Detectors are read-only: no project scripts, shell startup files, or install commands; a tool query (e.g. `brew --prefix`) is optional, allow-listed, bounded by a timeout, and its failure is reported rather than fatal. A conventional-path proposal does not depend on the tool being installed, so a leftover cache is still found after it is removed.

## v0.6.3

- Show Cargo build details directly in project trees, grouped by cleanup consequence: compiler caches, compiled tests and examples, and build-script output. Keep the directory view available without counting it as additional storage.
- Show candidate counts, allocated sizes, modification ages and removal consequences. Use available terminal space for candidate previews; improve Unicode alignment, narrow layouts and scrolling.
- Allow selecting a cleanup category or profile to review its supported groups. Profile selection does not delete the entire profile directory or silently include unsupported outputs.
- Run cleanup review and execution in background workers. Show progress, completed/refused counts and the current path; Escape or Ctrl-C stops between groups without interrupting an in-flight move. Cancelled reviews preserve the prior selection, and failed or unattempted cleanup selections remain available for review.
- Add a repository-specific AST audit against known blocking cleanup/review calls on TUI event and rendering paths, with regression fixtures in workspace tests.
- Document build details with a real screenshot and explain age, shared-link accounting, selection scope and Trash behavior. Age suggests what to review; it does not prove disuse. Dependencies remain a folded aggregate, not a per-crate size map.

## v0.6.2

- Allow reviewed Cargo groups containing hardlinks to move to same-filesystem Trash. Unselected links remain intact; uncertain reclaimed space no longer blocks cleanup. Content, membership, identity, lock and authorization checks remain in place.
- Isolate observations, history and replay checkpoints by canonical scan root, fixing proposals after switching between a project and its parent in one store. Root aliases share a scope; each new scope starts its own baseline.
- Add `cleanup-check --offset` paging and `--within` discovery scope, candidate totals, unchecked counts and coverage limits. Show hardlink/accounting warnings with results so a small checked page cannot be mistaken for the total cleanup opportunity.

## v0.6.1

- Added `cleanup-check`: a bounded review of individual Cargo groups that produces unapproved plans or specific refusals, without widening selections or deleting anything.
- Distinguished category totals, unchecked groups and blocked outputs in Rust reports, JSON and the Builds TUI. Rust text reports show 30 rows by default (`--all` restores the complete list), with accounting guidance before the rows.
- Replaced confusing Cargo category-selection errors with instructions to choose individual groups. Added hardlink and lock reason codes, retry guidance, exact reviewed members and recovery details.
- Kept existing hardlink, freshness, lock and human-approval protections. This release improves cleanup discovery; it does not add support for removing hardlinked groups or prove that old builds are unused.

## v0.6.0

- Added Rust build drilldown for Cargo profiles, folded dependencies, test/example executables, incremental-cache and build-script groups, and final executables/libraries. Group history does not inflate project totals. Dependency sizes remain a directory aggregate, not a per-crate breakdown.
- Added reviewed selective cleanup for evidenced test/example executables and individual incremental/build-script groups. Cleanup checks contents, producer evidence, locks, hardlinks and occupancy before moving the selection to Trash with restore metadata. Shared dependencies and final outputs remain inspection-only; age is not proof that a build is unused.
- Kept ordinary compiler files folded in reports and history, with trusted unchanged-container reuse instead of a persisted per-file inventory.
- Made incremental hardlinked-artifact refreshes update directory allocations without a whole-artifact rewalk. Unique-byte totals are explicitly marked stale until a full scan reconciles them; stale unique measurements create history gaps and cannot spend standing-grant budgets.
- Compacted small reverse deltas and delayed replay-checkpoint publication until report persistence succeeds.
- Fixed Cargo cleanup lock lifetime under concurrent process creation. Added native Cargo build/cleanup/rebuild verification and parallel stress coverage.

## v0.5.2

- Narrowed Ruby dependency attribution to `vendor/bundle`, preserving unrelated vendored source.
- Added declarative project-name fallbacks for Gradle settings, Cabal, and Python `setup.cfg` manifests.
- Made strict JSON manifest names structural and top-level only.
- Added regressions for shared Rust workspace targets and independent nested project targets.

## v0.5.1

- Fixed project badges with linked worktree counts so the worktree glyph and multi-digit count remain visually separated in the terminal UI.

## v0.5.0

- Fixed persistence of existing artifact byte changes in `current.parquet`. Added regression coverage for successive updates and unchanged observations after an update.
- Renamed the repository, source packages, binaries, environment variables, and release artifacts to `swamp`. The v0.5.0 archive contains `swamp` and `swamp-mcp`.
- Reorganized the documentation into a product overview, usage reference, architecture guide, and contributor guide. Corrected outdated UI, filter, installation, and history claims.

## v0.4.0

### Docker removal

- Added image and volume removal to the CLI, MCP, and TUI through Docker. This includes objects without project attribution.
- Added object-specific recovery information to plans, confirmations, and ledger records. Filesystem paths go to Trash; Docker removals do not. Images may be pulled or rebuilt if their sources remain available; swamp makes no copy of volume contents.
- Refused individual build-cache removal because the action path does not support that unit.
- Added live object checks before removal and surfaced Docker's refusal text.
- Separated trashed bytes, permanent removals, and measured free-space change in results.
- Added a Docker fixture with attributed and unattributed images, a volume, dangling images, build cache, and a container that prevents image removal.

### Git tracking and actions

- Split the worktree remainder into tracked `source`, `ignored`, and `untracked` buckets using directory-level Git status with large-file corrections. Apportioned totals preserve the walk's measured bytes.
- Prevented deletion of the ignored/untracked aggregate buckets as single paths.
- Reused the Git exclude stack within a checkout.
- Allowed direct project actions to select its actionable artifacts. When none exist, a direct action can offer the checkout; bulk marking skips that fallback.

### Terminal UI

- Replaced row sparklines with logarithmic change bars: growth extends right in red, shrink left in green, and changes below 1 MB use a small tick.
- Sorted growth by signed value, so increases precede decreases.
- Made Right open/expand and Left collapse/return, matching the displayed navigation.
- Fixed negative growth formatting, duplicated carried-forward GitHub signals, and empty container names.

### Storage and checks

- Changed report-history Parquet writes to close a temporary file before renaming it over the destination. This protects readers from unfinished individual files; it is not a multi-file transaction.
- Corrected source checks that matched their own explanatory comments and removed their ripgrep dependency.
- Corrected the fixture's dangling-image case on Docker installations using the containerd image store.

## v0.3.0

### Live and incremental observation

- Added a live FSEvents watch while the TUI is open, with observation after 400 ms of quiet.
- Retained directory detail inside folded artifacts so updates can re-list changed interior directories.
- Added a whole-artifact fallback for hardlinked units and local byte measurements to support deduplicated incremental accounting.
- Re-listed known remainder directories without walking their entire worktree when possible.
- Built one history index per growth-annotation pass and skipped unchanged current-file writes.
- Reused and aged Git signals for untouched worktrees, cached Docker facts for five minutes, and limited discovery around changed directories.

The release recorded these observations on one `~/src` tree with 55 projects and about 44 GB. Hardware details, repeated samples, and a reproducible benchmark harness were not supplied with the table.

| Case | v0.2.0 | v0.3.0 |
|---|---|---|
| Source file touched | 7.5 s | 75 ms |
| Change inside a 16 GB `target/` | 7.5 s | 2.4 s |
| No change | 7.5 s | 86 ms |

### Artifact recognition

- Added recognition of regular `CACHEDIR.TAG` files with the required signature.
- Vendored ignore/language lists and added a harvest utility to report candidate artifact names without editing the classification table.
- Expanded marker-gated recognition across ecosystems, including ESP-IDF, Godot, Jekyll, Elm, Erlang, OCaml, Clojure, and Nim.
- Added ESP-IDF `managed_components/` and build variants, CMake build variants, and Python `requirements*` markers.

### UI and reporting

- Changed growth to red and shrink to green; used a dark selection background to preserve those colors.
- Added history charts based on observed changes and ecosystem badges after project names.
- Preserved selection during background refresh.
- Included directory rows in the startup observation.
- Corrected progress counters and limited percentage display to cases where the previous total is a usable estimate.
- Added `--version` and release-version smoke checks.

## v0.2.0

- Added project ecosystem detection, marker-gated artifact classification, type filters and sorting, a types view, and manifest-based names for repositories without remotes.
- Added size and age predicates, project globs, more sorts, and persisted UI choices.
- Added the keep-executables option for supported Rust and Python outputs.
- Added observation progress and UI refresh after removal.
- Forced a full walk after classification-rule changes.
- Added time-series display with unobserved buckets represented separately from zero changes.
- Reorganized report construction into consumers on an in-memory event bus. See [ADR 001](docs/ADRs/001-event-bus-report-pipeline.md).
- Added source audits for selected implementation constraints and a mutation script for walker checks.
- Added configuration commands and MCP report filtering.

## v0.1.0

- Introduced a terminal UI, CLI, and stdio MCP server for disk growth by project, checkout/worktree, and artifact.
- Added Parquet history with reverse deltas, FSEvents-based incremental observation, and optional scheduled observation.
- Grouped clones by remote and exposed worktree activity, Git tracking, and cached GitHub facts.
- Added Docker attribution by Compose/source evidence, with unmatched objects reported as unowned. Docker removal arrived in v0.4.0.
- Added filesystem reconciliation and an optional `du` comparison.
- Added action plans, CLI approval and standing grants, execution, and ledger records.

The initial release targeted Apple silicon macOS with unsigned binaries. History began with the first observation. Performance figures were observations from one machine.
