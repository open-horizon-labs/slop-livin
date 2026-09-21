# Agent-tool storage

Coding-agent tools (Claude Code today; Codex/Oh My Pi/OpenCode/others
are researched and named but not yet identified -- see the matrix in
`docs/agent-storage.md`) keep session transcripts, caches, logs,
checkpoints and configuration under their own home directory. Swamp
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
has no supported action; `swamp propose-agents` refuses it by name,
never silently.

## Human keep/protect intent

```sh
swamp protect list --json
swamp protect add <path>       # survives refresh; blocks propose-agents for anything under it
swamp protect remove <path>
```

## Proposing agent-storage cleanup

Same rule as everywhere else in this skill: you may inspect and
propose, you must never approve or execute yourself.

```sh
swamp propose-agents --path <unit-path> --json
# -> {"state": "awaiting-authorization", "id": "...", "next_step": "a human authorizes with `swamp approve <id>` ..."}
```

`<unit-path>` is a unit's own `path` field from `--view agents`'
output -- an exact selection, never a category or the whole tool home.
Then the same lifecycle as every other plan:
`references/cleanup-and-recovery.md`'s propose -> approve -> execute
sequence, `swamp approve <id>` / `swamp execute <id> [--json]`, applies
unchanged (they are already generic over any plan).

If `propose-agents` refuses, the refusal names the exact reason
(`protected: ...`, `no supported selective action for <category> yet`,
`touches a database-like (SQLite/WAL/SHM) file`, or `refused: an active
process holds this path open`) -- relay it verbatim, never as "unsafe"
or "can't be deleted".

## Full reference

`docs/agent-storage.md` (in the repo, not this skill) has the complete
category table, project-linkage state table, the required 13-tool
matrix with sources, and every documented gap.
