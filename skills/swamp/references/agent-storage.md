# Agent-tool storage

Coding-agent tools (Claude Code, Codex and its desktop app, Oh My Pi,
OpenCode, Gemini CLI, Pi, Aider, GitHub Copilot CLI, Cursor, Windsurf,
Cline, Roo Code, Continue -- every tool in the required matrix, see
`docs/agent-storage.md`) keep session transcripts, caches, logs,
checkpoints and configuration under their own home directory (Aider's
per-repo files live inside each project checkout instead). Swamp
identifies this separately from ordinary project storage, links
sessions to the swamp project they belong to where evidence supports
it, and offers a narrow, supported cleanup path.

**Never read or repeat a session's actual content.** Swamp itself never
puts prompt/response/attachment/credential text into its output; you
must not either, even if you happen to see a path that looks
interesting. Talk about counts, bytes, ages, categories and project
links -- never contents.

## Inspect

```sh
swamp report --view agents --json                    # every identified unit
swamp report --view agents --project my-repo --json  # narrowed to one project's linked units
```

Each unit has a `category` (`sessions`, `caches`, `logs`, `checkpoints`,
`attachments`, `plugins`, `protected-config`, `unclassified`,
`managed-worktrees`), a `bytes`/`growth_bytes` pair, a `project_link`
(`linked`/`unresolved`/`missing`/`not-a-project`/`moved`/`remote`/
`shared`/`not-applicable`, never a basename guess), and `protected`
(true for credentials/config/skills/automation by default, or anything
a human added with `swamp protect`).

Only two categories have a supported action in this release:
**caches/logs** (a whole category directory, recoverable Trash move --
regenerated automatically by the tool) and **sessions** (the exact
transcript + its linked recovery material, moved together -- this
discards unique resume/rewind/checkpoint history, never the linked
project's own files). Everything else -- protected config, plugins
outside their own `.trash` staging area, attachments, unclassified --
has no supported action; `swamp propose --path <unit-path>` refuses it
by name, never silently.

A project's linked agent storage is also visible from the project tree
itself, not only `--view agents`: `swamp report --project <name>` (text
or `--json`, no `--view` needed) shows a collapsed "Agent storage
(linked)" summary alongside the project's worktrees.

## Human keep/protect intent

```sh
swamp protect list --json
swamp protect add <path>       # survives refresh; blocks propose/execute for anything under it
swamp protect remove <path>
```

## Proposing agent-storage cleanup

Same rule as everywhere else in this skill: you may inspect and
propose, you must never approve or execute yourself.

```sh
swamp propose --path <unit-path> --json
# -> {"state": "awaiting-authorization", "id": "...", "next_step": "a human authorizes with `swamp approve <id>` ..."}
```

`swamp propose` is the one entry point for every proposal kind: with no
`root`, a `--path` that matches a discovered agent-storage unit routes
here automatically (an external unit, or `--external` to force that
route explicitly, routes to inspection-only review instead -- see
`references/cleanup-and-recovery.md`). `swamp propose-agents --path
<unit-path>` still works too, as a deprecated alias into the identical
code path -- prefer `swamp propose` in new usage.

`<unit-path>` is a unit's own `path` field from `--view agents`'
output -- an exact selection, never a category or the whole tool home.
Then the same lifecycle as every other plan:
`references/cleanup-and-recovery.md`'s propose -> approve -> execute
sequence, `swamp approve <id>` / `swamp execute <id> [--json]`, applies
unchanged (they are already generic over any plan). A session removal
that partially fails (some members moved, then a later one could not
be) still names the Trash envelope and moved-byte count in the execute
result; a `restore.json` manifest inside that envelope records exactly
which member moved where, for manual recovery.

If `propose` refuses, the refusal names the exact reason
(`protected: ...`, `no supported selective action for <category> yet`,
`touches a database-like (SQLite/WAL/SHM) file`, `refused: an active
process holds this path open`, or `overlapping agent-storage
selections: ...` for two selected units that nest) -- relay it
verbatim, never as "unsafe" or "can't be deleted".

## Full reference

`docs/agent-storage.md` (in the repo, not this skill) has the complete
category table, project-linkage state table, the required 14-row tool
matrix with a per-tool support level and the upstream source each was
verified against, and every documented gap (Oh My Pi's shared-blob GC
and OpenCode's snapshot/`storage/part` actions are both deliberately out
of scope; Gemini CLI's project id is one-way). Twelve of the fourteen
rows are `supported`; **Cursor and Windsurf are `unverified`** -- their
units are identified and measured, but no action is offered and project
linkage is reported `unresolved`, because no primary source confirms the
layout those adapters model.
