# Agent-tool storage

Coding-agent CLIs and editor extensions (Claude Code, Codex, Oh My Pi,
OpenCode, and others) keep session transcripts, caches, logs,
checkpoints and configuration under their own home directory. This
document is the reference for how `swamp` discovers, models, presents
and selectively cleans up that storage (#90/#91/#92/#100/#101), and
what is deliberately not done yet.

**The aim:** help a developer understand where agentic coding tools
consume disk, what grew, which project (if any) it belongs to, and what
they can give up with an honest, specific consequence -- never a
"safe to delete" verdict, and never a guess.

## Privacy contract

This is the hard rule everything else in this document sits on top of:

- Identification reads directory names, file sizes, and modification
  times. For a session's project linkage, it reads **only the first
  line** of the session's own transcript file (bounded to 8 KiB),
  looking for one JSON field (`cwd`). Nothing else is ever read.
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
| Codex | Planned | `CODEX_HOME`, default `~/.codex` (`sessions/`, `auth.json`, `history.jsonl`, `logs/`, `config.toml`) | [openai/codex](https://github.com/openai/codex), [config-advanced](https://developers.openai.com/codex/config-advanced), [cli/reference](https://developers.openai.com/codex/cli/reference) |
| Oh My Pi | Planned | `~/.omp` (user-confirmed identity: a fork of `badlogic/pi-mono`) | [oh-my-pi/docs/session.md](https://github.com/can1357/oh-my-pi/blob/main/docs/session.md), [docs/settings.md](https://github.com/can1357/oh-my-pi/blob/main/docs/settings.md) |
| OpenCode | Planned | data: `OPENCODE_DATA_DIR`, else `${XDG_DATA_HOME:-~/.local/share}/opencode`; config: `${XDG_CONFIG_HOME:-~/.config}/opencode/opencode.json` | [opencode.ai/docs/troubleshooting](https://opencode.ai/docs/troubleshooting/), [opencode#6669](https://github.com/anomalyco/opencode/issues/6669), [opencode#18633](https://github.com/anomalyco/opencode/issues/18633) |
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
a tool *beyond* the thirteen named above is ordinary catalog review,
not a change to this contract.

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
  `-shm` sidecar (defense in depth; no currently-documented Claude Code
  path is SQLite, but the guardrail is unconditional regardless).
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
| `shared` | A unit's members collectively name more than one project. *Not currently populated by the Claude Code adapter* (one session always has exactly one declared `cwd`); kept for adapters/aggregates where it can genuinely happen. |
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

## Scan cost

Identification reads directory names and bounded metadata (file
`stat`, and a session's first transcript line) -- never a full
directory content walk with `crate::walk::resize_artifact`'s
parallel-pool machinery, which is tuned for a handful of potentially
huge artifact roots, not hundreds of small per-session directories.
`crates/core/src/agents/claude_code.rs`'s
`identification_cost_is_bounded_for_many_sessions` test measures
identifying 500 synthetic sessions (each with an oversized body, to
prove only the first line is ever read) and asserts it completes in
well under 10 seconds; see
`.oh/sessions/2026-09-21-agent-storage-claude-code.md` for the measured
number on the machine that ran it.

## Interfaces

| Surface | Command |
|---|---|
| CLI text | `swamp report --view agents [--project NAME] [--all]` |
| CLI JSON | `swamp report --view agents --json` (`{units, total_bytes}`) |
| TUI | `v` (cycle) reaches the read-only Agents view; no dedicated digit (`0` is "clear filter") |
| Protect | `swamp protect add\|remove\|list [--json] <path>` |
| Propose | `swamp propose-agents --path <unit-path> [--json]` |
| Approve/execute | `swamp approve <plan-id>` / `swamp execute <plan-id> [--json]` (unchanged -- already generic over any plan) |
| Skill | `skills/swamp/references/agent-storage.md` |

## Known gaps, recorded rather than hidden

- Only Claude Code has real identification code; the other twelve named
  tools are `Planned` (see the matrix above).
- TUI mark/confirm/execute for an agent-storage unit is not wired up;
  the CLI `propose-agents`/`approve`/`execute` path is the supported
  route this release.
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
