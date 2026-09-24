---
name: swamp
description: Investigate disk usage across a developer's Git projects with the swamp CLI -- what grew, which project/worktree it belongs to, build vs dependency vs Docker breakdown -- and explain what removing something would cost. Swamp reports; the human removes (in its TUI, or by hand) -- this tool never deletes anything. Use this whenever asked about disk space, what's using storage, what grew recently, stale build artifacts, or cleaning up a dev machine, when a `swamp` binary or `~/.local/share/swamp` store is present or mentioned.
---

# swamp: disk growth, by project

Swamp watches Git checkouts, linked worktrees, build output, dependency
trees, and Docker objects, and answers "what grew, where, and what would
it cost to remove it" -- never "what's safe to delete". It is a bounded
evidence tool, not a forensic oracle: absence of evidence for use is not
evidence of disuse.

**Swamp reports; the human removes.** The CLI and this skill are
entirely read-only: `swamp report` and its views print facts, and
`swamp protect` writes a human keep-list -- nothing else writes, and
nothing deletes. The only way anything gets removed is a human in
swamp's TUI (Space marks, Backspace shows current facts, Enter moves to
the Trash) or a human running a shell command themselves. There is no
`propose`/`approve`/`execute`/`grant` command any more; do not invent
one.

## Inspect first, always

Every session starts read-only, and stays that way -- there is nothing
past this to escalate to:

```sh
swamp scope --json                                      # what's in scope, and why -- check this first
swamp observe <root> --since 24h                         # the only command that scans; run this first
swamp report <root> --view grown --json                  # what grew, plus coverage
swamp report <root> --view projects --json             # ranked project list
swamp report <root> --view worktrees --json             # branch/idle/PR/merge facts
swamp report --view agents --json                       # Claude Code (etc.) session/cache/log storage
```

`swamp observe` is the *only* command that scans a filesystem or shells
out; `swamp report` is a pure read of whatever the last `observe` wrote
and never scans on its own. On a scope never observed, `report` prints
`no observation yet for <scope>; run swamp observe` (`--json`:
`{"error":"no_observation",...}`) and exits 2 -- run `observe` and
re-run `report`, never assume a scan happened implicitly.

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

## Explaining what removal would cost -- never proposing to do it

There is no plan, no approval, no execution and no grant left in
swamp: gather evidence and explain the consequence of removing
something (rebuild cost, redownload, lost session/checkpoint history,
lost emulator data, unpushed commits, no remote to restore from) in
plain words -- never a verdict like "safe to delete" or "unused", and
never framed as something you or swamp could do. If a human wants
something gone, tell them exactly how: open the TUI (Space the row,
Backspace to see the current facts, Enter to move it to the Trash), or
the precise path to remove by hand. Never claim to run, or offer to
run, a cleanup command -- none exists.

Each row/unit carries an `evidence` array (activity, consumer,
current-use, recovery, reclaimability facts with source and freshness)
-- read `references/evidence.md` before summarizing what keeping or
removing something would actually mean. What went where and how to get
it back after a human used the TUI: `references/cleanup-and-recovery.md`.

A build container (`target/`, `node_modules/`, a Gradle or Maven
`build/`) also carries identified units explaining what is inside it,
each stating its accounting basis, where its timestamp came from, and
what removing it would cost in that ecosystem's own words. Read
`references/build-artifacts.md` before answering "what is in my
node_modules" or "can I delete this"; several things that look
inferable there (a build generation, a package's identity from its
directory name, whether a Maven artifact can be downloaded again) are
deliberately not inferred.

## Why this needs no security boundary

There is nothing to guard against: no CLI command writes or deletes
anything except `swamp protect` (a keep-list) and the TUI's own Trash
move, both reachable only by a human at that keyboard. `swamp report`
and this skill cannot be talked into deleting something, because there
is no code path left that deletes anything from outside the TUI -- see
`references/trust-model.md` for the full statement of what swamp is now
and what changed.

## Reference index

Load a reference only when the task needs it -- this file alone is
enough for read-only investigation.

| Reference | Load it for | Measured size (`wc -c`) |
|---|---|---|
| `references/commands-and-json.md` | Full command/flag/JSON-schema reference including `swamp scope`, the historical MCP-tool-to-CLI-command mapping, exit codes | 11.4 KB |
| `references/cleanup-and-recovery.md` | Where a TUI Trash move went (ledger, envelope/`restore.json`) and how to restore it -- no CLI command deletes anything | 5.1 KB |
| `references/trust-model.md` | What swamp is now: read-only CLI/skill, TUI Trash as the only removal path, no authorization boundary to reason about | 4.6 KB |
| `references/coverage-and-history.md` | `since`/history-window resolution, partial/unknown coverage fields, reconciliation, scope/coverage-change notes, what a growth number does and doesn't prove | 6.8 KB |
| `references/filters.md` | The filter expression grammar (`kind:`, `growth >`, `idle >`, `merge-complete`, `pr:`, ...) | 2.8 KB |
| `references/agent-storage.md` | Coding-agent-tool storage (Claude Code sessions/caches/logs/protected config): categories, project linkage, `swamp protect` | 3.4 KB |
| `references/build-artifacts.md` | What is inside a build container (Cargo, Node, Gradle, Maven): role families, accounting basis, timestamp source, removal consequences, shared stores, and what is never inferred | 3.6 KB |
| `references/evidence.md` | The activity/consumer/current-use/recovery/reclaimability evidence contract: what each fact's `status`/`source`/`freshness` actually establishes, and what it does not | 5.2 KB |

This file is 5.8 KB (roughly 1,400 tokens at ~4 bytes/token). Each
reference loads independently -- none requires another to make sense,
and a read-only investigation task typically needs this file alone or
this file plus `commands-and-json.md`. If every reference were loaded
in the same turn (rare in practice) the total footprint is about
39 KB / ~9,700 tokens. These are measured byte counts, not a claim that
this beats any particular MCP client's own tool-schema overhead --
that overhead varies by client and was never measured here; see
`references/trust-model.md` for what this skill *does* claim about the
MCP-vs-CLI tradeoff (nothing about token cost, only about the
authorization boundary).
