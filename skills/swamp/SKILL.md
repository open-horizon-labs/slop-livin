---
name: swamp
description: Investigate disk usage across a developer's Git projects with the swamp CLI -- what grew, which project/worktree it belongs to, build vs dependency vs Docker breakdown -- and, when the human wants to reclaim space, propose a cleanup plan for their review. Use this whenever asked about disk space, what's using storage, what grew recently, stale build artifacts, or cleaning up a dev machine, when a `swamp` binary or `~/.local/share/swamp` store is present or mentioned.
---

# swamp: disk growth, by project

Swamp watches Git checkouts, linked worktrees, build output, dependency
trees, and Docker objects, and answers "what grew, where, and what would
it cost to remove it" -- never "what's safe to delete". It is a bounded
evidence tool, not a forensic oracle: absence of evidence for use is not
evidence of disuse.

## Inspect first, always

Every session starts read-only. Get oriented before proposing anything:

```sh
swamp scope --json                                      # what's in scope, and why -- check this first
swamp report <root> --view grown --json --since 24h   # what grew, plus coverage
swamp report <root> --view projects --json             # ranked project list
swamp report <root> --view worktrees --json             # branch/idle/PR/merge facts
```

`<root>` is the directory tree to scan (a `~/src`-style parent of
several checkouts, or one checkout) and is optional: omit it and
`report`/`observe`/`ui`/`schedule` resolve swamp's configured effective
scope instead (built-in roots, detected tool locations like Cargo/
rustup/Homebrew, and `config.toml`'s `[scan]` additions/exclusions).
`swamp scope --json` shows exactly what that resolves to, with
provenance for every root -- run it before trusting an implicit root.
Every command below is noninteractive and prints exactly one JSON
document to stdout with diagnostics on stderr -- safe to pipe to `jq`,
safe to run unattended. Full schemas, every view, pagination and
error/exit-code contract, and `swamp scope`'s own schema: see
`references/commands-and-json.md`. Narrowing what you see with
`--project`/`--filter`: see `references/filters.md`. What "since"/
"history"/partial coverage/scope actually mean before you trust a
growth number or an implicit root: see
`references/coverage-and-history.md`.

## Proposing cleanup

Swamp separates *evidence* from *authorization* from *action*. You
(the agent) may gather evidence and build a plan. You must never
approve or execute one yourself, and never run `swamp approve` or
`swamp grant add` on the human's behalf, even if asked to "just clean
it up" -- surface the plan and the exact command, and let the human run
it or explicitly tell you to run it in this session, once, as their own
typed instruction.

```sh
swamp propose <root> --filter 'kind:BuildOutput idle > 30d' --json
# -> {"state": "awaiting-authorization", "id": "...", "next_step": "a human authorizes with `swamp approve <id>` ..."}
```

Show the human the plan's units, bytes, and warnings (dirty checkout,
unpushed commits, no remote) verbatim -- they are facts to weigh, not
noise to summarize away. Full lifecycle (propose -> approve -> execute,
grants, `--keep-executables`, Trash recovery, refusal causes): see
`references/cleanup-and-recovery.md`.

## Why this isn't a security boundary

Nothing stops a shell-capable agent from typing `swamp approve` itself.
The rule above is behavioral guidance you follow, not a wall you are
behind. The actual safety boundary lives in swamp itself (the sink
re-checks every unit before touching disk, grants are scoped/budgeted/
expiring, every action is ledgered) -- see `references/trust-model.md`
before assuming "the agent can't authorize" means anything stronger than
"the agent chooses not to".

## Reference index

Load a reference only when the task needs it -- this file alone is
enough for read-only investigation.

| Reference | Load it for | Measured size (`wc -c`) |
|---|---|---|
| `references/commands-and-json.md` | Full command/flag/JSON-schema reference including `swamp scope`, the historical MCP-tool-to-CLI-command mapping, exit codes | 10.3 KB |
| `references/cleanup-and-recovery.md` | propose/approve/execute/grant lifecycle, Trash recovery, refusal causes, `cleanup-check` for Cargo builds | 5.1 KB |
| `references/trust-model.md` | The real authorization boundary: what the sink enforces vs. what is only behavioral convention | 4.6 KB |
| `references/coverage-and-history.md` | `since`/history-window resolution, partial/unknown coverage fields, reconciliation, scope/coverage-change notes, what a growth number does and doesn't prove | 4.2 KB |
| `references/filters.md` | The filter expression grammar (`kind:`, `growth >`, `idle >`, `merge-complete`, `pr:`, ...) | 2.8 KB |

This file is 5.3 KB (roughly 1,300 tokens at ~4 bytes/token). Each
reference loads independently -- none requires another to make sense,
and a read-only investigation task typically needs this file alone or
this file plus `commands-and-json.md`. If every reference were loaded
in the same turn (rare in practice) the total footprint is about
27 KB / ~6,700 tokens. These are measured byte counts, not a claim that
this beats any particular MCP client's own tool-schema overhead --
that overhead varies by client and was never measured here; see
`references/trust-model.md` for what this skill *does* claim about the
MCP-vs-CLI tradeoff (nothing about token cost, only about the
authorization boundary).
