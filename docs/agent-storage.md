# Agent-tool storage

Coding-agent CLIs and editor extensions (Claude Code, Codex, Oh My Pi,
OpenCode, Gemini CLI, Pi, Aider, GitHub Copilot CLI, Cursor, Windsurf,
Cline, Roo Code, Continue, and others) keep session transcripts,
caches, logs, checkpoints and configuration under their own home
directory (or, for Aider, partly inside each project checkout instead).
This document is the reference for how `swamp` discovers, models,
presents and selectively cleans up that storage
(#90/#91/#92/#93/#94/#95/#96/#97/#98/#99/#100/#101), and what is
deliberately not done yet. Every named tool in the required matrix now
has real identification code -- see "Required tool matrix" below.

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
  #96-#99's nine adapters follow the same discipline: Pi tries its own
  offset-zero header, then Oh My Pi's title-slot shape, for one `cwd`
  field, exactly like the tools it shares lineage with; Aider,
  Cursor/Windsurf and GitHub Copilot CLI's `session-store.db` never read
  transcript content at all (declared metadata or a protected,
  never-opened database); GitHub Copilot CLI's `session-state/` and
  Cline/Roo Code/Cursor/Windsurf's `workspace.json`/`task_metadata.json`
  linkage each read one small, bounded metadata file (never the
  conversation body) looking for one declared path field. Nothing else,
  in any adapter, is ever read.
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
  `plan_and_ledger_never_contain_the_canary_prompt_content`, and, for
  the nine #96-#99 adapters,
  `crates/core/tests/agent_units_actions_remaining_tools.rs`'s
  `no_canary_anywhere` helper, applied after every execute in that
  file).

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
| Gemini CLI | **Supported** | `~/.gemini`, or the whole of `GEMINI_CLI_HOME` when set (`settings.json`, `GEMINI.md`, `extensions/`, `trustedFolders.json`, `bin/`; `tmp/<hash>/{shell_history,checkpoints/,chats/}` and `history/<hash>/` shadow-Git repos, both keyed by `sha256(project root)`) | [configuration.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/reference/configuration.md), [checkpointing.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/checkpointing.md), [session-management.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/session-management.md), [paths.ts](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/utils/paths.ts) |
| Pi | **Supported** | `~/.pi/agent/`, overridable via `PI_CODING_AGENT_DIR` (confirmed primary-source name, shared with Oh My Pi's own override; `badlogic/pi-mono`, distinct from Oh My Pi which forks it) | [pi-mono settings.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/settings.md), [README.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/README.md) |
| Aider | **Supported** | `~/.aider/caches` (model-price/version-check caches); per-repo `.aider.chat.history.md`/`.aider.input.history` and `.aider.tags.cache.v{3,4}/` at the git root -- project-local, attached to the worktree artifact model, not the tool home | [models.py](https://github.com/Aider-AI/aider/blob/main/aider/models.py), [versioncheck.py](https://github.com/Aider-AI/aider/blob/main/aider/versioncheck.py), [args.py](https://github.com/Aider-AI/aider/blob/main/aider/args.py), [repomap.py](https://github.com/Aider-AI/aider/blob/main/aider/repomap.py) |
| GitHub Copilot CLI | **Supported** | `~/.copilot`, overridable via `COPILOT_HOME` (`config.json`, `settings.json`, `mcp-config.json`, `session-state/`, `command-history-state/`, `session-store.db`, `logs/`, ...); separate platform-conventional cache via `COPILOT_CACHE_HOME` | [cli-config-dir-reference](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference) |
| Cursor | **Supported** (macOS only) | editor-profile storage: `~/Library/Application Support/Cursor` (`User/globalStorage/state.vscdb`, `User/workspaceStorage/<id>/{state.vscdb,workspace.json}`, `User/History`), plus `~/.cursor/` (not decomposed) | community-sourced (no official layout doc found): [cursor-chat-browser](https://github.com/thomas-pedersen/cursor-chat-browser), [cursaves](https://github.com/Callum-Ward/cursaves/blob/main/docs/how-cursor-stores-chats.md) |
| Windsurf | **Supported** (macOS only, lower confidence) | `~/.codeium/windsurf` (not decomposed), plus an *assumed* VS-Code-fork profile at `~/Library/Application Support/Windsurf` -- not independently confirmed | [coder module](https://registry.coder.com/modules/coder/windsurf) (docs.windsurf.com redirects to docs.devin.ai as of this chunk) |
| Cline | **Supported** (macOS hosts) | VS Code extension global storage, one location per known editor host (Code, Code Insiders, Cursor, Windsurf, `~/.vscode-server` remote): `globalStorage/saoudrizwan.claude-dev/tasks/<task-id>/` | community-sourced: [cline#7101](https://github.com/cline/cline/issues/7101), [cline#14135](https://github.com/cline/cline/issues/14135) |
| Roo Code | **Supported** (macOS hosts) | Same per-host modeling as Cline: `globalStorage/rooveterinaryinc.roo-cline/tasks/<task-id>/` | community-sourced: [Roo-Code#4174](https://github.com/RooCodeInc/Roo-Code/issues/4174) |
| Continue | **Supported** | `~/.continue` (`config.yaml`/`config.json`, `sessions/<id>` + a session index file, `index/` embeddings/tag caches, `dev_data/` usage events) | [continue configuration](https://docs.continue.dev/customize/deep-dives/configuration), [reference](https://docs.continue.dev/reference) |

Every named tool in this matrix is now `Supported`; a test
(`matrix::tests::every_named_tool_is_now_supported`) enforces it. Two
rows are honestly narrower than the rest, stated rather than hidden:
Windsurf's editor-profile shape is *assumed* (VS Code fork), not
independently confirmed this chunk -- a real installation that differs
surfaces as an explicit "(unsupported layout version)" residual, never
a silent miscount; Cursor/Windsurf/Cline/Roo Code are macOS-only this
chunk, with Linux paths deferred to the independent Linux track
(#77-#89). Extending this list for a tool *beyond* the fourteen rows
above (Codex and its desktop app count separately, per #93's "do not
extrapolate one client's schema to all clients" acceptance) is ordinary
catalog review, not a change to this contract.

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

#96/#97/#98/#99's nine adapters needed **no new `AgentMemberKind`
variant**: Cursor/Windsurf's `state.vscdb` and GitHub Copilot CLI's
`session-store.db` reuse `Database`; Cline/Roo Code's task directories
and Copilot CLI's/Continue's directory-shaped session entries reuse
`SessionData`; every single-file session (Gemini CLI's saved chats,
Aider's per-repo history files, Pi's/Continue's per-session files, Copilot
CLI's file-shaped session entries) reuses `Transcript` -- direct
evidence that the shared model already covered these tools' shapes
without needing to be widened.

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

## Gemini CLI (#96)

Layout researched from `google-gemini/gemini-cli`'s own docs and source
(current `main` as of this chunk). The project-hash algorithm
(`getProjectHash(projectRoot) = sha256(projectRoot).hex()`) is confirmed
directly in `packages/core/src/utils/paths.ts` -- not guessed -- but it
is one-way: this adapter has no candidate project-path catalog to hash
and compare against, so every `tmp/<hash>`/`history/<hash>` unit's
`project_link` is `unresolved`, naming the algorithm explicitly, rather
than a fabricated match or a silently dropped fact.

- **Protected config:** `settings.json`, `GEMINI.md`, `trustedFolders.json`,
  `extensions/`. OAuth/account credential file names are not documented
  on any reachable page this chunk (`docs/cli/authentication.md` and
  `docs/get-started/authentication.md` both 404 against current `main`),
  so any top-level file whose name contains `oauth`/`cred` is protected
  defensively by filename pattern instead of an exact confirmed name.
- **Caches (actionable):** `bin/` (downloaded runtime tools, e.g.
  LiteRT-LM).
- **Per-project-hash `tmp/<hash>/`:** `shell_history` (Logs, actionable),
  `checkpoints/` (Checkpoints, not actionable -- tool-call recovery
  state for `/restore`), `chats/*` (Sessions, one unit per saved chat
  file, actionable -- `/chat save`/`/resume`).
- **`history/<hash>/`:** a shadow Git repository, independent of the
  project's own `.git`, backing the same `/restore` checkpoints. Unique
  recovery state, identified and linked (honestly unresolved), never
  offered a selective action this chunk.

## Pi (#96)

`badlogic/pi-mono`'s `coding-agent` package (also published as
`earendil-works/pi`) -- **distinct from Oh My Pi**, which is a fork of
it, even though both currently document the same override variable name
(`PI_CODING_AGENT_DIR`, confirmed by this chunk's own primary-source
read of Pi's README, correcting both this issue's own `PI_AGENT_DIR`
guess and superseding reliance on Oh My Pi's docs alone). If a human
sets that variable while both tools are installed, both detectors
resolve to the same path -- a disclosed, not silently patched,
limitation (see `crate::locations::pi`'s doc comment).

- **Explicit format/version detection:** Pi's own README documents
  session files only as JSONL with `id`/`parentId` tree structure -- it
  does **not** document Oh My Pi's 256-byte title slot. This adapter
  tries Pi's own offset-zero JSON header first, then Oh My Pi's
  title-slot-skip shape as an explicit fallback (reused, not assumed),
  and reports `unresolved` naming both shapes checked when neither
  matches.
- **Sessions:** `sessions/`, organized by working directory per Pi's own
  docs; this adapter does not decode a directory name into a project
  path (no encoding scheme is confirmed), relying only on each session
  file's own declared `cwd`.
- **Protected config:** `settings.json`, `trust.json`, `models.json`.
- **Caches (actionable):** `npm/` (user-scoped package installs,
  reinstallable).

## Aider (#96)

Materially different shape from every other tool in this catalog: most
of Aider's storage is **not** under any tool home at all.
`aider/args.py`/`repomap.py` (current `main` of `Aider-AI/aider`) place
`.aider.chat.history.md`, `.aider.input.history` and
`.aider.tags.cache.v{3,4}/` at each project's own git root. Per this
issue's explicit acceptance, these are attached to the existing
worktree/project model as an agent category -- **not** modeled as
tool-home units -- via `crate::agents::aider::identify_repo_units`,
called once per known project worktree root by
`crate::agents::discover_and_measure`'s `project_worktrees` parameter
(itself built from the already-loaded `Report`'s projects/worktrees at
the two read call sites, or, for `swamp propose-agents --path` which
computes no report, by walking upward from each requested path for its
own `.git` root).

- **Home-level (`~/.aider`):** `caches/model_prices_and_context_window.json`
  and `caches/versioncheck` (both wholly re-downloadable, confirmed
  directly in `aider/models.py`/`versioncheck.py`), plus an optional
  home-level `.aider.conf.yml`.
- **Per-repo (project-linked, `Sessions` category):**
  `.aider.chat.history.md`, `.aider.input.history` -- unique, not
  regenerated by re-running Aider.
- **Per-repo (project-linked, `Caches` category):**
  `.aider.tags.cache.v{3,4}/` (the version number reflects whether the
  optional TSL pack is in use; both are checked directly, not guessed at
  one fixed number) -- regenerated on the next Aider run.
- Disabling the `aider` detector (`disabled_detectors`) turns off *both*
  halves together, home-level and per-repo, so a human's "stop looking
  at this tool" always means the whole tool.

## GitHub Copilot CLI (#97)

Layout sourced directly from GitHub's own reference page (current as of
this chunk), correcting this issue's own guessed directory name
(`history-session-state/`) to the real `session-state/` and
`command-history-state/`.

- **Protected config:** `config.json`, `settings.json`, `mcp-config.json`,
  `lsp-config.json`, `permissions-config.json`, `providers.json`,
  `copilot-instructions.md`, `instructions/`, `agents/`, `hooks/`,
  `skills/`, `extensions/`, `installed-plugins/`, `plugin-data/`,
  `mcp-oauth-config/`, `mcp-secrets/`.
- **Sessions:** `session-state/` -- one unit per immediate child (file or
  folded directory), linked via a bounded, small-JSON scan for a
  `cwd`/`workspace`/`workspaceFolder` field in the child's own metadata
  files; `unresolved` when none is found, never a guess at an
  undocumented schema.
- **`session-store.db`** (+ `-wal`/`-shm`): protected, non-actionable,
  same discipline as every other tool's cross-session SQLite store.
- **Caches (actionable):** `command-history-state/` (reverse-search
  command recall, not conversation content).
- **Logs (actionable):** `logs/`.
- **`ide/`** (IDE integration state/lock files): identified but never
  actionable this chunk -- a lock file backing an active integration is
  a real corruption risk, and no documented signal distinguishes an idle
  entry from a live one.
- **Cache, separately:** platform-conventional (`~/Library/Caches/copilot`
  on macOS), independent of `COPILOT_HOME`, overridable via
  `COPILOT_CACHE_HOME`; reported as an opaque external unit.

## Cursor and Windsurf (#98)

Both use the shared `crate::agents::vscode_family` module (Cursor and
Windsurf are VS Code forks with the same underlying storage
conventions -- one real implementation, per the handoff's "extend the
shared model only where a tool genuinely needs a new concept").
Community-reverse-engineered (Cursor: corroborated by two independent
sources; Windsurf: assumed, since `docs.windsurf.com` redirected to
`docs.devin.ai` during this chunk's research and no primary
documentation of its layout was reachable). Both are macOS-only this
chunk; Linux is the independent Linux track's job (#77-#89).

- **`User/globalStorage/state.vscdb`** (+ `-wal`/`-shm`): protected,
  metadata-only, never opened while writable -- holds every project's
  chat/composer content (`ItemTable`/`cursorDiskKV` key-value stores).
- **`User/workspaceStorage/<id>/state.vscdb`**: a per-workspace index
  (not the content itself), linked via the sibling `workspace.json`'s
  `folder` field (a real `file://` URI the editor itself wrote, not a
  basename guess) -- also protected and non-actionable, but carries real
  project linkage as identification evidence.
- **`User/History/`** (actionable): local file-history/undo snapshots,
  unrelated to AI chat content.
- **`Cache/`, `CachedData/`, `CachedExtensionVSIXs/`** (actionable
  caches) and **`logs/`** (actionable log): Electron-conventional
  siblings of `User/`.
- **Unrecognized layout:** one explicit "(unsupported layout version)"
  residual, never a guess -- this is how a Windsurf installation that
  does not actually match the assumed VS-Code-fork shape shows up,
  rather than a silent miscount.
- Cursor's separate `~/.cursor/` and Windsurf's `~/.codeium/windsurf`
  are reported as opaque external units, not decomposed (no confirmed
  interior shape).

## Cline, Roo Code and Continue (#99)

Cline and Roo Code are VS Code **extensions** (not forks): the same
extension id can be installed into several editor hosts at once, each
with its own, genuinely separate, on-disk `globalStorage`. Per this
issue's explicit acceptance, `crate::locations::vscode_hosts` proposes
one location *per known host* (Code, Code Insiders, Cursor, Windsurf,
plus `~/.vscode-server` for a remote/devcontainer target), and
`crate::agents::discover_and_measure` decomposes *every* resolved
location for these two tool ids (`multi_location_tool`), unlike every
other tool in this catalog, which only decomposes the first. Both
adapters reuse `crate::agents::vscode_family::identify_extension_globalstorage`,
which tags every unit's `relative_path` with its host label (e.g.
`"VS Code/tasks/<id>/..."`, `"Cursor/tasks/<id>/..."`) so two hosts'
task directories -- which can share the exact same UUID-shaped name --
never collide in identity or display.

- **Sessions:** `globalStorage/<extension-id>/tasks/<task-id>/`, folded
  as one unit per task. Linked via `task_metadata.json`'s `workspace`
  field -- this chunk's own research (from the issue text), not
  independently re-confirmed against a schema doc; `unresolved` when
  missing, never a guess.
- Roo Code's own community reports note a task directory can embed a
  full Git checkpoint repository, so its folded byte total is not
  necessarily small the way a Claude Code session usually is --
  `SessionRemoval`'s existing generic loss warning already covers this.

Continue is unrelated to the VS Code storage conventions above --
its own `~/.continue` home, config confirmed by primary docs:

- **Protected config:** `config.yaml`/`config.json`.
- **Sessions (actionable):** `sessions/<id>` -- one unit per entry,
  excluding index-like filenames (`sessions.json`/`index.json`), which
  are protected separately instead (removing the index alongside a kept
  session would otherwise corrupt it for every session that remains). No
  confirmed per-session workspace-linkage field was found, so every
  session carries an honest `unresolved` link rather than a guess from
  the session id.
- **Caches (actionable):** `index/` (embeddings/tag caches).
- **Logs (actionable):** `dev_data/` (anonymized usage events).
- `sessions/index/dev_data`'s presence is treated as a version marker
  (this catalog's own prior research, not independently re-confirmed
  this chunk) rather than an asserted schema.

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
| `claude_code` | ~215ms (500 synthetic sessions, 200KB bodies each) |
| `codex` | ~209ms |
| `oh_my_pi` | ~368ms (includes the bounded per-session blob-reference scan) |
| `opencode` | ~6ms |
| `gemini_cli` | ~161ms (300 synthetic project-hash directories) |
| `pi` | ~203ms |
| `aider` | ~6ms (`identify_repo_units` over a 2000-file tags cache) |
| `copilot_cli` | ~172ms |
| `vscode_family` (Cursor/Windsurf/Cline/Roo Code) | ~132ms (500 synthetic tasks) |
| `continue_dev` | ~5ms |

See `.oh/sessions/2026-09-21-agent-storage-claude-code.md` for the
Claude Code number's original recording,
`.oh/sessions/2026-09-21-agent-storage-codex-omp-opencode.md` for
Codex/Oh My Pi/OpenCode, and
`.oh/sessions/2026-09-21-agent-storage-remaining-tools.md` for the nine
tools this chunk added.

## Interfaces

| Surface | Command |
|---|---|
| CLI text | `swamp report --view agents [--project NAME] [--all]` |
| CLI JSON | `swamp report --view agents --json` (`{units, total_bytes}`) |
| TUI (read) | `v` (cycle) reaches the Agents view; no dedicated digit (`0` is "clear filter") |
| TUI (act) | `Space`/`Backspace` mark the selected agent unit and open the confirm banner (`App::mark_row`'s agent-storage branch); `Shift+A` (`mark_all_in_view`) marks every actionable row in the Agents view the same way, skipping protected/unsupported/active ones and naming the skip in the footer; `Enter` executes through the ordinary background-worker path (`execute_plan_progress`), never blocking the event/render thread. A protected/unsupported row cannot be marked; the footer names `propose_agents`'s own refusal reason. |
| Protect | `swamp protect add\|remove\|list [--json] <path>` |
| Propose | `swamp propose --path <unit-path> [--json]` (no `root`) -- the unified entry point (#101): routes to the agent-storage proposer when a path matches a discovered agent unit, else to the external-unit proposer, else (only with a `root`) the ordinary filesystem proposer. `swamp propose --external [--path <unit-path>] [--json]` forces the inspection-only external-unit route explicitly. `swamp propose-agents --path <unit-path> [--json]` still works, as a thin, deprecated alias into the exact same code path (prints a one-line deprecation note to stderr). |
| Approve/execute | `swamp approve <plan-id>` / `swamp execute <plan-id> [--json]` (unchanged -- already generic over any plan) |
| Skill | `skills/swamp/references/agent-storage.md` |
| Project tree | `swamp report --project <name>` (text) and `--project <name> --json` (no `--view` needed) both include this project's linked agent storage: a collapsed "Agent storage (linked)" row per contributing tool in the text tree (`crate::tree::agent_rows_for_project`, shared by the CLI drill and the TUI's own Tree view), and an `agent_storage: {units, total_bytes}` object in the JSON envelope. |

## Known gaps, recorded rather than hidden

- Every named tool in the matrix now has real identification code (see
  the matrix above) -- the epic's full-catalog acceptance is met at the
  identification/project-linkage layer; independent validation (#102)
  is still a separate, unchecked box.
- TUI bulk marking (`Shift+A`, `mark_all_in_view`) now recognizes agent
  rows too (reusing `App::mark_row`'s own per-row protected/unsupported/
  active refusal, never a duplicated refusal path): the actionable rows
  in view are marked, and a footer names how many were skipped and why
  when at least one was. Marking one agent unit at a time
  (`Space`/`Backspace` on the selected row) still works exactly as
  before.
- **Resolved this chunk:** `swamp propose --path` (and the deprecated
  `propose-agents` alias) used to skip computing a full `Report` for
  speed, discovering Aider's per-repo units only by walking upward from
  each requested path's own worktree root
  (`crate::agents::worktree_root_containing`). That meant a path whose
  worktree root was not itself part of the request could not surface an
  Aider unit at all. The unified `propose --path` route now runs the
  same real report walk `swamp report --view agents` does (every known
  project worktree, not just the ones implied by the requested paths)
  before resolving agent units, closing this gap at the cost of a full
  scope walk instead of a handful of `stat`s -- deliberate, since
  `propose` without a `root` is not a hot path. See
  `discover_agent_units_for_propose` in `crates/cli/src/main.rs`.
- Windsurf's editor-profile shape and the `task_metadata.json`
  `workspace` field Cline/Roo Code use for project linkage are this
  chunk's own best-available research, not independently re-confirmed
  against an official schema/layout document -- both degrade to an
  honest "unresolved"/"(unsupported layout version)" outcome rather than
  a wrong guess when they do not match a real installation.
- Cursor, Windsurf, Cline and Roo Code are macOS-only this chunk; their
  Linux paths are the independent Linux track's job (#77-#89), not
  re-derived here as a guess.
- **Resolved this chunk:** `swamp propose`'s `root` is now `Option<PathBuf>`.
  Omitting it and passing `--path` routes to the agent-storage proposer
  or the external-unit proposer automatically (see the Interfaces table
  above); `--external` forces the latter explicitly. `propose-agents`
  remains only as a thin, deprecated alias so existing scripts keep
  working.
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
  their sections above, not silently treated as settled facts. This
  chunk corrected two more env var guesses against primary source:
  Gemini CLI's real override is `GEMINI_CLI_HOME` (not
  `GEMINI_CONFIG_HOME`), and Pi's real override is `PI_CODING_AGENT_DIR`
  (not `PI_AGENT_DIR`, which is still honored defensively as an
  unconfirmed secondary override).
- GitHub Copilot CLI's `ide/` and Aider's home-level `.aider.conf.yml`/
  residual entries are identified but never actionable this chunk (the
  former: real corruption risk from touching an active integration's
  lock file with no documented idle signal; the latter: simply out of
  named scope) -- an explicit `None` action, not a missing feature
  disguised as empty.

## #100/#101 completion and #102 validation (this chunk)

This chunk audited every #100/#101 acceptance line against the
implementation left by the prior four chunks (Claude Code; Codex/Oh My
Pi/OpenCode; the remaining nine tools), fixed the gaps it found, and
added the independent #102 validation pass. Landed:

- **Project-tree agent linkage (#100 gap).** `crate::tree::ProjectTree`
  gained `agent_rows: Vec<ProjectAgentToolRow>` (`crate::tree::
  agent_rows_for_project`), a per-tool collapsed summary of the agent
  storage linked to one project. `render_project_tree_with_agents`
  (CLI text) and `model::tree_rows_with_agents` (TUI Tree view) both
  render one "Agent storage (linked)" line per contributing tool,
  informational only (`unit: None` in the TUI -- acting on a specific
  unit stays the dedicated Agents view's job, with its own per-unit
  protections/occupancy checks). Previously this information existed
  only in the flat `--view agents` list, with no way to see "does this
  project have any linked agent storage" from the project drill itself.
- **`report --project NAME --json` agent linkage (#100 gap).** The
  project-scoped JSON envelope (the `None`-view branch of
  `report_json_envelope`) now includes an `agent_storage: {units,
  total_bytes}` object whenever agent units were computed, not only
  when `--view agents` was also passed. `agent_units` is now computed
  whenever `view == Some(View::Agents) || project.is_some()`.
- **Unified `propose` entry point (#101).** `swamp propose`'s `root` is
  now `Option<PathBuf>`; a `--path` with no `root` is routed to the
  agent-storage proposer (if it matches a discovered agent unit), else
  the external-unit proposer, else refused by name (never silently
  falling through to a filesystem interpretation it cannot support
  without a root). `--external` forces the external-unit route
  explicitly, satisfying the "propose --external ... inspection-only
  refusal at execution" requirement B2 had left at the Rust API level
  with no CLI surface. `propose-agents` is now a thin, deprecated alias
  that delegates into the identical routing function
  (`propose_unified` in `crates/cli/src/main.rs`) and prints a one-line
  deprecation note.
- **Aider discovery fixed in the unified route (chunk E follow-up).**
  The old `propose-agents --path` fast path discovered Aider's per-repo
  units only by walking upward from the requested paths themselves,
  which could not find an Aider unit whose worktree root was not
  implied by the request. The unified route now runs the same real
  report walk `report --view agents` uses to supply every known project
  worktree first.
- **Refusal matrix hardening (#101).** `propose_agents` now refuses an
  agent-storage plan whose selected units' own paths nest (parent/child
  overlap), the same discipline `propose`'s Cargo-group check already
  applied to filesystem units -- previously nothing enforced this for
  agent units. A table-driven test
  (`crates/core/tests/agent_refusal_matrix.rs`) exercises every named
  refusal reason (protected category, human protect flag, unsupported
  action, database-like path, active session, whole-home/whole-
  projects-dir path, overlap, plan scope drift at execution) across all
  14 named tool ids.
- **Recoverable moves and partial-failure accounting (#101).** Session
  removal now writes a `restore.json` recovery manifest into its Trash
  envelope before any member moves, and rewrites it after each
  successful move -- so a partial failure (some members moved, then a
  later rename fails) leaves an accurate, on-disk record of exactly
  what happened, and `execute`'s own outcome now names the envelope and
  the bytes that really did move (`PartialAgentRemoval` in
  `crates/core/src/actions.rs`) instead of only a bare error string. All
  agent-storage actions remain Trash moves (recoverable), never a
  permanent native operation.
- **#102 validation suite.** `crates/core/tests/agent_storage_validation.rs`
  (custom-root redirection, malformed/truncated metadata across three
  adapters, unknown-schema-never-actionable across three adapters,
  shared-blob reference-state correctness, an integrated canary sweep
  across render text/JSON/plan/execute-result/ledger for two
  differently-shaped adapters, nested-accounting agreement between the
  flat Agents view and the project tree, incremental/unchanged-refresh
  growth history, and stable history after relinking a session to a
  different project) and
  `crates/tui/tests/agent_storage_validation.rs` (a real, on-disk
  fixture run through real identification into the TUI's own rendered
  frames -- both the Agents view and the new project-tree row -- to
  prove real session content never reaches what a human actually sees,
  not just the JSON/plan/ledger surfaces the prior chunks' tests
  already covered).
- **Benchmarks** (recorded in
  `.oh/sessions/2026-09-21-agent-storage-validation.md`): 300 synthetic
  Claude Code sessions (each padded past 200 KB so only a bounded read
  is ever exercised) scan in tens of milliseconds; an unchanged refresh
  and a one-session append cost about the same as the initial scan
  (this adapter re-scans its tool home each call rather than
  incrementally diffing it -- see that session note for why this is an
  honest, not-yet-optimized fact, not a regression).

## Human review still needed

Per this chunk's own instruction not to claim human review happened:
**no human has reviewed any of the following.** They are exactly what
Muness (or another maintainer) should check before treating this
feature as fully validated, carried forward from prior chunks' session
notes and `FOLLOWUPS.md` rather than silently resolved by guessing:

- **Retention-consequence wording.** Do the loss-of-resume/rewind
  warnings (`unit_from_agent`'s `warnings` in `crates/core/src/actions.rs`,
  and this doc's own "Session removal" section) actually read as clear
  and honest to a human deciding whether to delete a session, or are
  they too technical, too reassuring, or missing a consequence a real
  user would care about? Not evaluated by any automated test.
- **Unresolved format assumptions**, verbatim from
  `FOLLOWUPS.md`'s own tracking note (chunk E): "Windsurf layout
  assumed; Cline/Roo Code `task_metadata.json` workspace field
  unconfirmed; Gemini OAuth credential filename unconfirmed;
  Cursor/Windsurf/Cline/Roo Code macOS-only paths (Linux track)." None
  of these were resolved this chunk -- each still degrades to an honest
  "unresolved"/"(unsupported layout version)" outcome rather than a
  wrong guess when a real installation does not match, but whether the
  *assumption itself* is correct needs a maintainer with an actual
  installation of the tool in question (or a reachable primary-source
  layout doc this chunk could not find) to confirm.
- **Other named unknowns, verbatim from `FOLLOWUPS.md`**: "`~/.claude.json`
  sibling file and `todos/` prefix heuristic are named unknowns"
  (Claude Code); "Codex `skills/`/`log/` names + `CODEX_SQLITE_HOME`;
  OpenCode `OPENCODE_DATA_DIR` env var name" are unverified; "Oh My Pi
  blob GC and OpenCode snapshot/`storage/part` removal deliberately not
  offered (complete-reference requirement)" remains a scope boundary,
  not a bug.
- **No real installed tool has ever been checked.** Every fixture in
  this entire feature (all four chunks plus this one) is synthetic, by
  the hard privacy rule the handoff sets. That means no adapter's
  format assumptions have been verified against an *actual* `~/.claude`,
  `~/.codex`, `~/.omp`, or any other real tool home on any real machine
  -- only against primary-source documentation/code and this project's
  own hand-built fixtures. Whether the real, currently-installed version
  of each tool on Muness's own machine actually matches what this
  adapter expects is exactly the kind of outside check the handoff asks
  to be surfaced, not fabricated. A reasonable first step: run `swamp
  report --view agents` against a real machine with one or two of these
  tools installed and compare the reported categories/sizes against
  what `du`/`ls` show by hand for the same directory -- without ever
  feeding a real transcript's content into a bug report, fixture, or
  test.
- **Linux paths** for Cursor/Windsurf/Cline/Roo Code (and re-verification
  of every other adapter's Linux path) are explicitly out of scope for
  this macOS-only chunk; the independent Linux track (#77-#89) owns
  that work, not a guess made here.
- **Whether the epic can actually close.** #90/#102's own acceptance
  requires every named tool tested *and* independent validation *and*
  human review of usefulness. The first two are now substantially
  covered (the per-adapter test files, this chunk's refusal-matrix and
  validation suites); the third -- an actual human reading actual
  output and judging whether it is useful, not vague, not overwhelming
  with caveats, and not misleading -- has not happened and cannot be
  simulated by an agent. This is the honest remaining blocker to
  closing #90, not a technical gap.
