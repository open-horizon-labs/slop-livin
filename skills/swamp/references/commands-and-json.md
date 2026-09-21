# Commands and JSON contract

Every command in this reference is noninteractive: it reads (and, for
`report`/`observe`, writes) the growth store under `$SWAMP_DIR`
(default `~/.local/share/swamp`), prints exactly one JSON document to
stdout when `--json` is given, and never prompts. Diagnostics, progress
lines, and parse errors go to stderr only -- stdout is safe to pipe
straight into `jq` or a JSON parser, and a nonzero exit means stdout is
empty (no partial/malformed JSON document is ever printed).

This table replaces the MCP server that shipped through v0.6.x
(`crates/mcp`, removed in favor of this CLI + skill). Every MCP tool
below maps onto CLI flags that reuse the exact same core logic
(`swamp_core::agent_json`), so results are identical in content, not
just similar in spirit.

| Former MCP tool | CLI equivalent |
|---|---|
| `report` (root, since, project, view, filter, dirs) | `swamp report <root> --json [--view V] [--project P] [--filter F] [--since S] [--dirs]` |
| `what_grew` (root, since) | `swamp report <root> --view grown --json --since <S>` |
| `list_projects` (root, since) | `swamp report <root> --view projects --json [--since S]` |
| `list_worktrees` (root, since, filter) | `swamp report <root> --view worktrees --json [--filter F]` |
| `docker_objects` (root, unowned_only, project) | `swamp report <root> --view docker --json [--unowned-only] [--project P]` |
| `propose` (root, since, filter, paths) | `swamp propose <root> --json [--since S] [--filter F] [--path P ...]` |
| `execute` (plan_id, keep_executables) | `swamp execute <plan_id> --json [--keep-executables]` |
| `plans` | `swamp plans --json` |
| `grants` (read-only) | `swamp grant list --json` |

There is no CLI equivalent for an MCP tool that *writes* a grant,
because none ever existed: grant creation was always
`swamp grant add`/human-at-CLI only. See `trust-model.md`.

`<root>` is now optional on `report`/`observe`/`ui`/`schedule`: omit it
and the command resolves swamp's configured effective scope (built-in
defaults, detected tool locations, and `config.toml`'s `[scan]` table)
instead of one explicit path. `swamp scope --json` (below) is the one
place every one of those commands' root resolution is inspectable.

## `swamp scope [<root>...] --json`

No MCP predecessor -- new in the scope/detector-registry work (#41/#44).
Prints the effective scan scope: every root swamp would use for this
invocation (or, given explicit roots, what those resolve to -- config
`exclude` still applies), each with its filesystem status
(`present`/`missing`/`unreadable`/`skipped-as-nested`/`excluded`) and
every reason it is in scope, plus the full detector catalog (including
`disabled`/`not-present`/`unresolved-with-reason` entries never
promoted to a root) and the catalog version:

```json
{
  "catalog_version": "2026-09-21.1",
  "generated_at": 1758470400,
  "defaults_enabled": true,
  "disabled_detectors": [],
  "configured_include": [],
  "configured_exclude": [],
  "explicit": false,
  "roots": [
    {
      "path": "/Users/you/src",
      "reasons": [{"source": "detector", "detector_id": "builtin-defaults", "category": "unclassified", "provenance": {"BuiltinConvention": null}}],
      "status": {"state": "present"}
    }
  ],
  "detectors": [
    {"detector_id": "cargo-home", "name": "Cargo home", "locations": [{"detector_id": "cargo-home", "path": "/Users/you/.cargo", "category": "installation", "provenance": "builtin-convention", "status": {"state": "resolved"}, "note": "cargo home: bin/, config.toml, credentials"}]}
  ],
  "pruned_subtrees": []
}
```

An effective scope with no roots at all (`defaults = false`, no
`include`, every detector disabled) is not an empty `roots: []` --
`report`/`observe`/`scope` all refuse to run with a nonzero exit and an
explicit stderr message, never a silent fallback to the current
directory. See [coverage-and-history.md](coverage-and-history.md) for
the coverage-change notes `report`/`observe` print when the resolved
scope differs from the last observation.

## `swamp report <root> --json`

Always applies `--filter` (if given) to the whole report before
computing any view -- a filtered view and a filtered whole-report
agree on what rows exist. `--project` scopes to one project (matched by
name or `owner/repo` display name). Without `--view`, prints the full
report structure with `since`/`index_refreshed`/`total`/`truncated`
added and the top-level `projects` array bounded by `--limit`/
`--offset`.

### `--view <name> --json`

Every view returns the envelope:

```json
{
  "view": "worktrees",
  "project": null,
  "result": [ /* view-specific: array or object */ ],
  "observed_at": 1234567890,
  "since": "24h",
  "index_refreshed": true,
  "total": 12,
  "truncated": false
}
```

`total`/`truncated` are present whenever `result` is an array; they
describe the array's *unbounded* length and whether `--limit`/
`--offset` cut anything off this page -- never assume a page is the
whole answer without checking `truncated`. `since` is the window the
call actually used (your `--since`, else `config.toml`'s, else the
1h/24h/7d default) -- not just your raw argument, so a caller always
knows what window its numbers reflect. `index_refreshed` is `false`
only under `--no-observe`.

Views:

| `--view` | `result` shape | Notes |
|---|---|---|
| `worktrees` (default) | array: `{project, path, branch, idle_secs, merge_complete: {verdict, terms}\|null, pull_request, remove_command}` | `remove_command` is text for a human to run, never executed by swamp. `merge_complete` is a composite fact with its terms, never a verdict. |
| `projects` | array: `{name, display_name, project_id, bytes, growth_bytes, checkout_count, worktree_count, remote}` | Ranked growth desc, then bytes desc. JSON only (no text render). |
| `grown` | object: `{grown: [...], unowned_by_reason: {...}, permission_denied_count}` | See below; JSON only. |
| `builds` | array: `{project, kind, path, bytes, growth_bytes}` | `BuildOutput` + `Cache` kinds. |
| `deps` | array: same shape | `DependencyTree` kind only. |
| `docker` | array: `{project, object, kind, bytes, shared_bytes, created_at, shared_with, containers, dangling, note, unowned}` | Add `--unowned-only` to restrict to objects with no join evidence. `project` is `null` unless `--project` is given, in which case unowned rows are labelled `"<name> (unowned, name-alike)"` -- never silently attributed. |
| `kinds` | array: `{kind, bytes, count}` | |
| `types` | object keyed by ecosystem | |
| `unowned` | array: `{path_or_object, bytes, reason, shared_bytes, docker_kind, note}` | |
| `reconciliation` | object: `{attributed, unowned, walked_total, du_total, docker_attributed, docker_unowned}` | |
| `rust` | array of nested Cargo artifacts | Inspection only; not project-scoped by `--project` yet. |

`--view grown` additionally has a top-level `coverage` block:

```json
"coverage": {
  "walked_total": 123, "du_total": null, "unowned_total": 0, "attributed_total": 123,
  "observed_at": 1234567890, "since": "24h", "index_refreshed": true,
  "history": {
    "history_secs": 0, "asked_window_secs": 3600, "effective_window_secs": 0,
    "note": "asked for 3600s of growth but the store holds 0s of observations; growth is reported over 0s"
  }
}
```

`history.note` is set whenever the asked window exceeds what the store
actually holds, or when the store holds no history at all ("no
observations yet: growth cannot be reported") -- a growth number
without checking this is a number that may describe a shorter window
than you asked for. See `coverage-and-history.md`.

`--view projects`/`--view grown` are JSON-only: in text mode (no
`--json`) they exit nonzero with a stderr message instead of silently
falling back to a different view.

### Pagination

`--limit N` / `--offset N` bound the array in `result` (or the
top-level `projects` array with no `--view`). Both are only consulted
with `--json`. A truncated page still reports the true `total`; never
treat a `--limit`-bounded call as an exhaustive inventory.

## `swamp propose <root> --json`

```json
{
  "id": "...", "root": "...", "created_at": 0, "expires_at": 0,
  "status": "Proposed", "units": [ {"kind": "...", "path": "...", "bytes": 0,
    "growth_bytes": null, "recovery": "local_rebuild", "verb": "delete",
    "warnings": ["dirty"], "...": "..."} ],
  "refused": [ {"path": "...", "cause": "..."} ],
  "state": "awaiting-authorization",
  "planned_bytes": 0,
  "next_step": "a human authorizes with `swamp approve <id>` (this plan) or a standing `swamp grant add ...`; then run `swamp execute <id>` (add --json for machine output). Proposing never authorizes removal.",
  "observed_at": 1234567890
}
```

Proposing never deletes or authorizes anything, ever -- `state` is
always `awaiting-authorization` on a fresh plan. Plans expire (default
30 minutes) and are single-use.

## `swamp execute <plan_id> --json`

Returns `ExecuteResult`: `state` (`executed` | `awaiting-authorization`
| `expired` | `already-executed`), `next_step`, `outcomes` (per-unit
`status`: `completed` | `refused` | `failed`, with `cause` when not
completed), `planned_bytes`, `trashed_bytes`,
`removed_permanently_bytes`, `freed_measured`. An unauthorized plan
executes with `state: "awaiting-authorization"` and an empty
`outcomes` array -- nothing is touched. See `cleanup-and-recovery.md`
for refusal causes and Trash recovery.

## `swamp plans --json` / `swamp grant list --json`

```json
{"plans": [ /* Plan objects, newest first */ ], "total": 3}
{"grants": [ /* Grant objects */ ], "total": 1,
 "note": "grants are minted only by a human running `swamp approve <plan_id>` or `swamp grant add ...`; no command reads standing authorization into existence on its own"}
```

## Exit codes and error semantics

- `0`: the command ran and, in JSON mode, printed exactly one JSON
  document to stdout.
- Nonzero (currently always `1`): something failed before any JSON was
  printed -- a bad `--filter` expression, an unreadable root, a missing
  plan id, an I/O error. Diagnostics are on stderr; stdout is always
  empty in this case. Never parse stdout on a nonzero exit.
- A *refused* or *awaiting-authorization* outcome (a plan with no
  grant, a unit whose activity changed since proposal, an execute on an
  expired plan) is not an error: the command exits `0` and the refusal
  is a fact inside the JSON body (`state`, `outcomes[].status`,
  `outcomes[].cause`). Check the body, not just the exit code, before
  assuming an action happened.
