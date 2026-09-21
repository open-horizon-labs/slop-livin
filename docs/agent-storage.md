# Agent-tool storage

Coding-agent CLIs and editor extensions (Claude Code, Codex, Oh My Pi,
OpenCode, and others) keep session transcripts, caches, logs,
checkpoints and configuration under their own home directory. This
document is the reference for how `swamp` discovers, models, presents
and selectively cleans up that storage
(#90/#91/#92/#93/#94/#95/#100/#101), and what is deliberately not done
yet.

**The aim:** help a developer understand where agentic coding tools
consume disk, what grew, which project (if any) it belongs to, and what
they can give up with an honest, specific consequence -- never a
"safe to delete" verdict, and never a guess.

## Privacy contract

This is the hard rule everything else in this document sits on top of:

- Identification reads directory names, file sizes, and modification
  times. For most adapters, a session's project linkage comes from
  reading **only the first line** of the session's own transcript file
  (bounded to 8 KiB), looking for one JSON field (`cwd`); Oh My Pi's
  sessions have a fixed 256-byte title slot before that header line and
  this adapter skips it rather than reading it. OpenCode's project
  linkage instead comes from its own small, declared `project.json`
  metadata file (a `worktree` field), never from session content at
  all. Oh My Pi's shared-blob reference accounting is the one adapter
  that reads further into a session body (bounded to 64 KiB per
  session), and only to extract opaque `blob:sha256:<hash>` reference
  tokens -- never any other content, never persisted as text anywhere.
  Nothing else, in any adapter, is ever read.
- No prompt, response, attachment, tool-output, or credential *content*
  is put into a report, a plan, the ledger, a log, or a test fixture.
  Every fixture used to build and test this feature is synthetic:
  invented session ids, invented project paths, placeholder bodies
  like `"[redacted]"`.
- Formats were researched from primary upstream documentation/source
  (linked below), never learned by reading a real `~/.claude` (or any
  other real agent home) on a development machine.
- Adversarial tests seed a canary string into fixture session bodies
  and assert it never appears in any unit, plan, execute result, or
  ledger entry (`crates/core/src/agents/claude_code.rs`'s
  `a_session_is_identified_with_its_companion_members`,
  `crates/core/tests/agent_units_actions.rs`'s
  `plan_and_ledger_never_contain_the_canary_prompt_content`).

## Required tool matrix

Source of truth: `crates/core/src/agents/matrix.rs` (`MATRIX`). This
table is kept in sync with it by hand; a test
(`matrix::tests::every_named_tool_is_present_exactly_once`) enforces
that every one of the required tools has exactly one row.

| Tool | Status | Home / override | Sources |
|---|---|---|---|
| Claude Code | **Supported** | `~/.claude`, or `$CLAUDE_CONFIG_DIR` if set | [claude-directory](https://code.claude.com/docs/en/claude-directory), [settings](https://code.claude.com/docs/en/settings), [checkpointing](https://code.claude.com/docs/en/checkpointing), [authentication](https://code.claude.com/docs/en/authentication) |
| Codex | **Supported** | `CODEX_HOME`, default `~/.codex` (`sessions/`+`archived_sessions/` year/month/day rollout trees, `auth.json`, `history.jsonl`, `config.toml`, `log/`, six `*.sqlite` state stores relocatable via the separate `CODEX_SQLITE_HOME`) | [home-dir/lib.rs](https://github.com/openai/codex/blob/main/codex-rs/utils/home-dir/src/lib.rs), [rollout/lib.rs](https://github.com/openai/codex/blob/main/codex-rs/rollout/src/lib.rs), [rollout/list.rs](https://github.com/openai/codex/blob/main/codex-rs/rollout/src/list.rs), [rollout_file_name.rs](https://github.com/openai/codex/blob/main/codex-rs/rollout/src/rollout_file_name.rs), [rollout/metadata.rs](https://github.com/openai/codex/blob/main/codex-rs/rollout/src/metadata.rs), [state/sqlite.rs](https://github.com/openai/codex/blob/main/codex-rs/state/src/sqlite.rs), [codex_home_metrics.rs](https://github.com/openai/codex/blob/main/codex-rs/app-server/src/codex_home_metrics.rs) |
| Codex desktop app | **Supported** (log directory only) | macOS `~/Library/Logs/com.openai.codex`; settings/session storage beyond logs is unconfirmed and not modeled | [doctor/desktop.rs](https://github.com/openai/codex/blob/main/codex-rs/cli/src/doctor/desktop.rs), [desktop/platform.rs](https://github.com/openai/codex/blob/main/codex-rs/cli/src/doctor/desktop/platform.rs) |
| Oh My Pi | **Supported** | `~/.omp/agent`, or the whole of `PI_CODING_AGENT_DIR` when set (user-confirmed identity: a fork of `badlogic/pi-mono`) | [oh-my-pi/docs/session.md](https://github.com/can1357/oh-my-pi/blob/main/docs/session.md), [docs/settings.md](https://github.com/can1357/oh-my-pi/blob/main/docs/settings.md) |
| OpenCode | **Supported** | data: `OPENCODE_DATA_DIR` (unconfirmed env var name, honored defensively), else `${XDG_DATA_HOME:-~/.local/share}/opencode`; config `${XDG_CONFIG_HOME:-~/.config}/opencode` and cache `${XDG_CACHE_HOME:-~/.cache}/opencode` reported as opaque external units, not decomposed | [opencode.ai/docs/troubleshooting](https://opencode.ai/docs/troubleshooting/), [opencode#6669](https://github.com/anomalyco/opencode/issues/6669), [opencode#18633](https://github.com/anomalyco/opencode/issues/18633), [storage.ts](https://github.com/sst/opencode/blob/dev/packages/opencode/src/storage/storage.ts), [DeepWiki storage-and-database](https://deepwiki.com/sst/opencode/2.9-storage-and-database) |
| Gemini CLI | Planned | `~/.gemini` (`settings.json`, `GEMINI.md`, `extensions/`, `tmp/`, `chats/`); root overridable via `GEMINI_CONFIG_HOME` | [gemini-cli configuration.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/reference/configuration.md) |
| Pi | Planned | `~/.pi/agent/`, overridable via `PI_AGENT_DIR` (`badlogic/pi-mono`, distinct from Oh My Pi, which forks it) | [pi-mono settings.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/settings.md), [development.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/development.md) |
| Aider | Planned | `~/.aider` (`~/.aider/caches`); per-repo `.aider.chat.history.md`/`.aider.input.history` and `.aider.tags.cache.v<N>/` are project-local, not under the home directory | [aider config.html](https://aider.chat/docs/config.html), [usage/caching.html](https://aider.chat/docs/usage/caching.html), [Aider-AI/aider](https://github.com/Aider-AI/aider) |
| GitHub Copilot CLI | Planned | `~/.copilot`, overridable via `COPILOT_HOME` | [cli-config-dir-reference](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference) |
| Cursor | Planned | editor-profile storage, not one directory: macOS `~/Library/Application Support/Cursor/User/{globalStorage,workspaceStorage}`, plus `~/.cursor/` (chats/projects/CLI) | community-sourced (no official layout doc found): [cursor-chat-browser](https://github.com/thomas-pedersen/cursor-chat-browser), [cursaves](https://github.com/Callum-Ward/cursaves/blob/main/docs/how-cursor-stores-chats.md) |
| Windsurf | Planned | `~/.codeium/windsurf` (MCP/agent config); editor chat/workspace storage layout unconfirmed | [docs.windsurf.com](https://docs.windsurf.com/), [coder module](https://registry.coder.com/modules/coder/windsurf) |
| Cline | Planned | VS Code extension global storage: `.../globalStorage/saoudrizwan.claude-dev/tasks/<task-id>/` | community-sourced: [cline#7101](https://github.com/cline/cline/issues/7101), [cline#14135](https://github.com/cline/cline/issues/14135) |
| Roo Code | Planned | VS Code extension global storage: `.../globalStorage/rooveterinaryinc.roo-cline/tasks/<task-id>/` | community-sourced: [Roo-Code#4174](https://github.com/RooCodeInc/Roo-Code/issues/4174) |
| Continue | Planned | `~/.continue` (`config.yaml`/`config.json`, `sessions/`) | [continue configuration](https://docs.continue.dev/customize/deep-dives/configuration), [reference](https://docs.continue.dev/reference) |

"Planned" means the home-path evidence above was researched from
primary sources during this work, but no identification code exists
for that tool yet -- never a silent omission, and never an empty
adapter that claims support it does not have. Extending this list for
a tool *beyond* the fourteen rows above (Codex and its desktop app
count separately, per #93's "do not extrapolate one client's schema to
all clients" acceptance) is ordinary catalog review, not a change to
this contract.

## Categories

Every identified unit falls into one of these (`AgentCategory` in
`crates/core/src/agents/mod.rs`):

| Category | Meaning | Protected by default? | Supported action |
|---|---|---|---|
| Sessions | Transcripts and their directly-linked recovery material (companion subagent dir, checkpoints, todos, attachments) | Individually (e.g. `history.jsonl`) where an adapter says so | **Session removal** -- exact member set, reference-verified fresh at execution |
| Attachments | Shared or per-session pasted/uploaded content | Often, when shared/not session-scoped | None yet |
| Checkpoints | File-history/backup material not currently linked to any live session | No (unless orphaned and adapter-flagged) | None yet |
| Caches | Regenerated-automatically technical caches | No | **Cache/log Trash move** |
| Logs | Regenerated-automatically debug/diagnostic logs | No | **Cache/log Trash move** |
| Managed worktrees | Git worktrees a tool created | N/A -- reuses existing worktree identity, never separately re-measured | Existing Git/worktree action protections apply, not this module's |
| Plugins | Marketplace configuration and downloaded plugin/skill code | No (the `.trash` staging subdirectories are actionable; the rest is not) | **Cache/log Trash move** for `.trash` only |
| Protected config | Credentials, settings, skills, commands, subagent/automation definitions | **Yes, always** | None -- never actionable |
| Unclassified | Anything with no specific rule (a residual bucket, never silently dropped) | Case by case | None yet |

Only **cache/log Trash move** and **session removal** have a supported
selective action in this release. Everything else is identification
only: it appears in a report, it can be inspected, and it is
respected by `swamp protect`, but there is no code path that deletes
it. This matches the guardrail this feature was built under: identity
and action capability are independent facts, and the absence of one is
never disguised as the other.

`AgentMember.kind` (`AgentMemberKind`) gained two variants for #93/#94/
#95, both explicitly sanctioned by the handoff's "extend the shared
model only where a tool genuinely needs a new concept" clause:

- **`Database`** -- a SQLite file (or a `-wal`/`-shm` sidecar) backing
  a newer version's unified session/message/state store (Codex's six
  `*.sqlite` files, OpenCode's `opencode.db`, Oh My Pi's `agent.db`).
  Always folded into one unit with its sidecars as members; never split,
  never opened, never individually actionable (`is_sqlite_like` in
  `crate::actions` refuses any selective action on a path with this
  kind unconditionally).
- **`SessionData`** -- a session-keyed companion directory that is
  neither a transcript, a subagent dir, todos, file-history, nor an
  attachment (OpenCode's `storage/message/<session-id>/` and
  `storage/session_diff/<session-id>/`), matched to a session by the
  same exact-id-match discipline `claude_code::identify` uses for its
  own companion directories -- never a guess.

### Cache/log Trash move

A single directory (e.g. `debug/`, `shell-snapshots/`, `statsig`,
`plugins/.trash`, `skills/.trash`) is moved into Trash as one unit,
recoverable until Trash is emptied. The tool regenerates these
automatically; removing them costs nothing except the next
regeneration.

### Session removal

A session's exact member set -- the transcript itself, its companion
subagent directory, its `file-history/<session>/` checkpoint data, and
any `todos/`/`image-cache/`/`uploads/` entries matched to it -- is
moved together into one Trash envelope. This is **not** a
rebuildable-cache action: it discards unique resume/rewind/checkpoint
history for that session. The linked project's own files are never
touched -- only the tool's own record of the conversation is affected.
Before acting, `execute` re-derives the session's current membership
from scratch and refuses if it has drifted (a member is now missing, a
new member the plan did not know about has appeared, or the whole
session is no longer identifiable at that path) rather than acting on
a stale plan.

### What is refused, always

- Whole tool-home deletion. An `AgentUnit` never represents the whole
  home; there is nothing to select that would delete it.
- The whole `projects/` directory, or a whole project's session
  directory as one unit -- only individual sessions are units.
- Any path whose filename looks like a SQLite database or its `-wal`/
  `-shm` sidecar. Claude Code has none currently documented, but the
  guardrail is unconditional regardless, and Codex/OpenCode/Oh My Pi
  each have real SQLite stores it actually protects (six state
  databases, `opencode.db`, `agent.db`).
- A content-addressed blob Oh My Pi's `blobs/` shares across sessions:
  identified with its reference count (or an explicit "coverage
  unknown" note when any session's body exceeded this pass's bounded
  scan), but never offered a selective action at all in this release --
  reference-based GC from incomplete coverage is out of scope here, not
  merely gated.
- OpenCode's `snapshot/<project-id>/` git-backed checkpoint store and
  its `storage/part/` (message parts, keyed by message id, not session
  id): identified and linked where possible, never offered a selective
  action -- removing a snapshot loses `/undo` history, and parts cannot
  be safely correlated to one session without reading message content.
- An active session: `swamp propose-agents`/`execute` both check
  occupancy (an `lsof`-style check on the session's transcript file,
  via the same `crate::occupancy` seam existing actions use) and refuse
  while a process holds it open.
- Anything a human protected with `swamp protect add <path>`.

## Project linkage

Each unit's `project_link` field is one of:

| State | Meaning |
|---|---|
| `linked` | Resolved to a swamp project/worktree identity (`crate::git`'s own object-store-based identity -- never a filesystem path or a basename match), with `declared` or `inferred` provenance. |
| `unresolved` | No project metadata could be extracted at all (malformed/empty transcript header, or the `cwd` field was missing). |
| `missing` | Metadata names a path that no longer exists on disk. |
| `not-a-project` | Metadata names a path that exists but is not (or is no longer) a Git checkout/worktree. |
| `moved` | The declared path used to resolve to one project identity and now resolves to a different one (or none). *Not currently populated by the Claude Code adapter* -- it has no record of a session's previous linkage to compare against. |
| `remote` | The declared path is on a different host than this observation runs on. *Not currently populated* -- no reliable remote-host signal exists in a session header. |
| `shared` | A unit's members collectively name more than one project. *Not populated by the Claude Code, Codex or OpenCode adapters* (one session always has exactly one declared `cwd`/`worktree`); the Oh My Pi adapter does populate it, when a session's `additionalDirectories` names a workspace root resolving to a different project than its primary `cwd`. |
| `not-applicable` | This unit is inherently tool-wide (a cache, a log directory, protected config): project linkage does not apply, which is a different, more honest fact than "we tried and could not tell." |

Resolution never decodes the `projects/<encoded-cwd>` directory name
back into a path: that encoding (slashes replaced with a separator) is
lossy in the reverse direction, since a literal hyphen in a real path
cannot be told apart from an encoded separator. The only reliable
source is the session transcript's own declared `cwd` field, walking
upward from that path for a `.git` directory/file using the same
identity primitives `crate::git::discover` uses for ordinary project
scanning.

Bidirectional navigation:

- **Project → agent storage:** `swamp report --view agents --project
  <name>` narrows to units whose linkage names that project.
- **Agent unit → project:** every unit's own `project_link` field
  names its project (or explains why it cannot).

Linked bytes are never added to the linked project's own filesystem
total: an agent unit lives under the tool's external-unit home, a
separate accounting basis, shown as a reference, not summed in.

## Claude Code (#92)

Layout researched from Anthropic's own documentation (linked in the
matrix above); the transcript JSONL schema itself is explicitly
documented upstream as internal and unstable across versions, so this
adapter reads at most one field (`cwd`) off a session's first line and
treats anything else as an unknown, never a parse panic or a guess.

- **Sessions:** `projects/<encoded-cwd>/<session-id>.jsonl`, its
  companion `projects/<encoded-cwd>/<session-id>/` directory
  (subagents/tool-results), `file-history/<session-id>/`, and matching
  `todos/`, `image-cache/<session-id>/`, `uploads/<session-id>/`
  entries. The `todos/` match is a **documented heuristic** (filename
  prefix match on the session id): the exact naming convention is not
  in Claude Code's official documentation. Session ids are UUIDv4, so
  collision risk is negligible; an ambiguous match is excluded rather
  than guessed.
- **Caches (actionable):** `shell-snapshots/`, `statsig` (community-
  documented, not found in the official settings fetch used here),
  `plugins/.trash/`, `skills/.trash/`.
- **Logs (actionable):** `debug/`.
- **Protected config:** `settings.json`, `.credentials.json`,
  `keybindings.json`, `themes/`, `rules/`, `skills/`, `commands/`,
  `agents/`, `workflows/`, `output-styles/`, `agent-memory/`.
- **Individually protected, not by category:** `history.jsonl` (every
  prompt typed, kept for recall/search -- category `sessions`, but
  flagged protected because its content is prompt text and it has no
  supported action).
- **Not modeled, a documented gap:** `~/.claude.json` lives *beside*
  `~/.claude/`, not inside it. An agent unit's identity is a path
  under the tool home by construction; a sibling file does not fit
  that model, so it is left out rather than forced in.
- **Not separately re-measured:** git worktrees Claude Code creates
  (`--worktree`, `EnterWorktree`, `isolation: worktree`). Claude Code
  documents no fixed on-disk location for them; they are ordinary Git
  worktrees the normal project scan already discovers and measures.
  This adapter cross-references via project linkage rather than
  inventing a second measurement of the same bytes.
- **Active-session detection:** an `lsof`-style occupancy check on a
  session's transcript file (`crate::occupancy::occupied`), run only
  when proposing/executing an action on that specific unit -- never
  during ordinary identification, which would mean hundreds of process
  spawns on an otherwise-cheap `report`.

## Codex (#93)

Layout researched from `openai/codex`'s own `codex-rs` source (current
`main` as of this chunk; see the matrix above for exact file links).
`CODEX_HOME`'s internal transcript envelope around the `session_meta`
entry's `cwd` field is not pinned to one nesting shape (genuinely
version-varying wire format); any shape not matched resolves to
`unresolved`, never a guess.

- **Sessions:** `sessions/<year>/<month>/<day>/rollout-<timestamp>-
  <thread-id>[_<rollout-id>].jsonl` -- one file per session, no
  documented companion directory, so a session's member set is always
  exactly that one file.
- **Archived sessions:** `archived_sessions/` in the same date-tree
  shape, identified the same way with an explicit "archived, not
  evidence of disuse" note -- archiving is a Codex-side visibility
  change, not a deletion.
- **SQLite state stores (a version boundary):** `state_5.sqlite`,
  `logs_2.sqlite`, `goals_1.sqlite`, `memories_1.sqlite`,
  `queue_1.sqlite`, `thread_history_1.sqlite`, each folded with its
  `-wal`/`-shm` sidecars into one protected, non-actionable unit.
  `CODEX_SQLITE_HOME` can relocate all six *outside* `CODEX_HOME`; this
  adapter does not follow that override (a documented gap, same shape
  as Claude Code's `~/.claude.json` sibling gap) -- if set, these files
  are simply not found here rather than guessed at a wrong path.
- **Protected config:** `config.toml`, `auth.json`, `skills/`
  (directory name found in source search, not independently confirmed
  by a primary docs page this chunk).
- **Individually protected, not by category:** `history.jsonl`
  (cross-session prompt history, category `sessions`).
- **Logs (actionable):** `log/` (name carried over from this epic's
  prior research, not independently re-confirmed by source this
  chunk).
- **Not confirmed, not modeled:** no managed-worktree creation by the
  CLI itself was found in this chunk's source research, so
  `AgentCategory::ManagedWorktrees` is never populated by this adapter
  -- an honest absence, not a silent gap.

### Codex desktop app (#93)

The desktop app (`Codex.app`, bundle id `com.openai.codex`) is a
materially different client with its own storage; this chunk does
**not** extrapolate the CLI's `CODEX_HOME` schema onto it. Only its log
directory is confirmed by primary source
(`codex-rs/cli/src/doctor/desktop.rs`'s `desktop_log_root`): macOS
`~/Library/Logs/com.openai.codex`, itself a `%Y/%m/%d` date tree. That
directory is identified as one folded, actionable Logs-category unit.
Settings/session storage beyond logs is not confirmed and is not
modeled -- a deliberately partial `Supported` row, stated explicitly
rather than silently treated as empty. No Linux/Windows desktop build
is confirmed either; the detector reports `not-present` there instead
of guessing a path.

## Oh My Pi (#94)

User-confirmed identity: a fork of `badlogic/pi-mono`'s `pi` coding
agent. Layout researched from `can1357/oh-my-pi`'s own docs (current
`main` as of this chunk).

- **Unknown-format disambiguation:** `~/.omp` is also a plausible home
  for unrelated tools (the issue names oh-my-posh as one to check).
  Resolving `~/.omp/agent` specifically (not the bare `~/.omp` wrapper)
  already avoids most collision risk, and this adapter adds a second,
  independent check on top: before identifying anything, it looks for
  at least one of this format's own content markers (`config.yml`,
  `config.yaml`, `agent.db`, `sessions/`, `blobs/`). Absent all of
  them, it reports one non-actionable "unknown format" unit for the
  whole directory rather than guessing.
- **Sessions:** `sessions/<encoded-cwd>/<timestamp>_<session-id>.jsonl`
  -- files begin with a fixed 256-byte `type: "title"` slot, then the
  session header (`cwd`, `additionalDirectories`); this adapter skips
  the title slot and reads only the header line, same one-field-at-a-
  time discipline as every other adapter here.
- **Shared content-addressed blobs:** `blobs/<sha256>`, referenced from
  session bodies as `blob:sha256:<hash>`. Establishing *complete*
  reference coverage would mean reading every session body in full,
  which this adapter deliberately does not do: each session's body is
  scanned only up to a bound (64 KiB), extracting reference tokens
  only -- never persisted or logged as text. A session exceeding the
  bound marks the whole pass's blob-reference coverage as unknown
  rather than reporting a possibly-wrong count. No blob is ever offered
  a selective action in this chunk regardless of its reference count --
  reference-based GC is out of scope here, not merely gated.
- **Terminal breadcrumbs (actionable):** `terminal-sessions/`.
- **Protected config:** `config.yml`/`config.yaml`, `models.yml`,
  `agent.db` (a SQLite auth store, doubly protected -- by category and
  by `is_sqlite_like`).
- **Project-local, not modeled:** `<cwd>/.omp/config.yml` lives outside
  the agent home entirely (same class of documented gap as Aider's own
  project-local files in the matrix above).

## OpenCode (#95)

Layout researched from `sst/opencode`'s own source and DeepWiki-indexed
documentation (current as of this chunk). Unlike the other three tools,
OpenCode keeps data, config and cache as three *independent* roots
(`crate::locations::opencode`); only the data root is decomposed into
`AgentUnit`s -- config and cache are reported as opaque external units
with their own byte totals, since neither carries session/project
linkage.

- **Version-aware boundary:** `identify` checks for
  `opencode.db`/`storage/`/`snapshot/`/`auth.json`/`log/` before doing
  anything else. A resolved, non-empty data root matching none of them
  reports one "unsupported layout version" unit rather than guessing
  at either schema below.
- **Newer/SQLite layout:** `opencode.db` (+ `-wal`/`-shm`), folded into
  one protected, non-actionable unit -- never opened, same discipline
  as Codex's own SQLite stores.
- **Older/file-tree layout:** `storage/session/<project-id>/
  <session-id>.json`, with `storage/message/<session-id>/` and
  `storage/session_diff/<session-id>/` companions matched by the exact
  session-id-keyed discipline `claude_code::identify` uses for its own
  companions. `storage/part/<message-id>/*.json` is keyed by *message*,
  not session, id and is never correlated to individual sessions
  without reading message content -- folded whole into one protected,
  non-actionable unit instead, the same honest-gap discipline Claude
  Code's own `paste-cache` uses.
- **Project linkage, declared and cheap:** `storage/project/
  <project-id>.json`'s `worktree` field is a real filesystem path, read
  once per project directory and reused for every session under it --
  no session-body scan needed at all for this adapter's project
  linkage, unlike every other adapter in this document.
- **Git-backed checkpoint snapshots (both layouts):**
  `snapshot/<project-id>/<hash>` -- an internal git object store,
  decoupled from the project's own `.git`, capturing a tree snapshot
  before/after every agent step so `/undo` can revert. Unique
  checkpoint/recovery state, identified and linked, never offered a
  selective action in this chunk.
- **Protected config:** `auth.json` (directly under the data root, not
  the separate config root).
- **Logs (actionable):** `log/` (also directly under the data root).

## Scan cost

Identification reads directory names and bounded metadata (file
`stat`, and a session's first transcript/header line) -- never a full
directory content walk with `crate::walk::resize_artifact`'s
parallel-pool machinery, which is tuned for a handful of potentially
huge artifact roots, not hundreds of small per-session directories.
Each adapter has its own `identification_cost_is_bounded_for_many_sessions`
test, measuring identification of 500 synthetic sessions and asserting
completion in well under 10 seconds:

| Adapter | Measured (this chunk's dev machine) |
|---|---|
| `claude_code` | ~75ms |
| `codex` | ~68ms |
| `oh_my_pi` | ~110ms (includes the bounded per-session blob-reference scan) |
| `opencode` | ~8ms |

See `.oh/sessions/2026-09-21-agent-storage-claude-code.md` for the
Claude Code number's original recording, and
`.oh/sessions/2026-09-21-agent-storage-codex-omp-opencode.md` for the
other three.

## Interfaces

| Surface | Command |
|---|---|
| CLI text | `swamp report --view agents [--project NAME] [--all]` |
| CLI JSON | `swamp report --view agents --json` (`{units, total_bytes}`) |
| TUI (read) | `v` (cycle) reaches the Agents view; no dedicated digit (`0` is "clear filter") |
| TUI (act) | `Space`/`Backspace` mark the selected agent unit and open the confirm banner (`App::mark_row`'s agent-storage branch); `Enter` executes through the ordinary background-worker path (`execute_plan_progress`), never blocking the event/render thread. A protected/unsupported row cannot be marked; the footer names `propose_agents`'s own refusal reason. |
| Protect | `swamp protect add\|remove\|list [--json] <path>` |
| Propose | `swamp propose-agents --path <unit-path> [--json]` |
| Approve/execute | `swamp approve <plan-id>` / `swamp execute <plan-id> [--json]` (unchanged -- already generic over any plan) |
| Skill | `skills/swamp/references/agent-storage.md` |

## Known gaps, recorded rather than hidden

- Five of the fourteen matrix rows have real identification code
  (Claude Code, Codex, the Codex desktop app, Oh My Pi, OpenCode); the
  remaining nine named tools are `Planned` (see the matrix above).
- TUI bulk marking (`Shift+A`, `mark_all_in_view`) does not reach agent
  rows yet -- it only recognizes a `row.kind`/`ArtifactKind` or a
  projects-view `row.project`, neither of which an agent row sets.
  Marking one agent unit at a time (`Space`/`Backspace` on the selected
  row) works; a future worker can extend `mark_all_in_view` to agent
  rows the same way it already handles projects-view rows.
- `swamp propose`'s CLI (the original, walked-report-root command) has
  no `--agent` mode; `propose-agents` is a separate, dedicated
  subcommand instead (mirrors the same reasoning the External chunk
  recorded for its own CLI gap: relaxing `Propose`'s required `root:
  PathBuf` is a larger, riskier change than an additive new command).
- Plugins/marketplace removal beyond the `.trash` staging directories
  is not supported; native marketplace-aware removal is the preferred
  future mechanism, not a guessed directory delete.
- `paste-cache/` is treated conservatively (protected, no action) even
  though Claude Code's own retention policy treats it as ephemeral,
  because it is not scoped to one session and this adapter has no
  per-session reference evidence for it.
- Oh My Pi's shared blobs and OpenCode's git-backed snapshots/`storage/
  part` are identified and linked where possible but never offered a
  selective action in this chunk (see their sections above) -- this is
  a deliberate scope boundary, not an oversight: reference-based blob
  GC and per-hash snapshot removal both need reference/coverage
  guarantees this chunk does not implement.
- Codex's `CODEX_SQLITE_HOME` and OpenCode's `OPENCODE_DATA_DIR` exact
  env var name are both documented as unconfirmed/partially-followed in
  their sections above, not silently treated as settled facts.
