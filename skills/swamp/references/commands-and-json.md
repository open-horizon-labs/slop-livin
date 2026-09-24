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

There is no `propose`/`execute`/`plans`/`grant` command any more (removed
2026-09-23, "swamp reports; the human removes"): the CLI's only
side-effecting command is `swamp protect add|remove` (a human keep-list).
Nothing deletes anything except the TUI's own Trash flow, or a human's
own shell command. See `trust-model.md`.

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
promoted to a root) and the catalog version. `disabled_detectors` is
every detector not running this pass; `default_off_detectors` (stack/26)
is the subset of those off because the detector itself defaults to off
(a system-wide install tree -- currently only Homebrew) rather than
because your config named it -- `[scan] enabled_detectors = ["homebrew"]`
turns it back on:

```json
{
  "catalog_version": "2026-09-21.4",
  "generated_at": 1758470400,
  "defaults_enabled": true,
  "disabled_detectors": ["homebrew"],
  "default_off_detectors": ["homebrew"],
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
  "pruned_subtrees": [],
  "external_pruned_subtrees": []
}
```

`detectors` covers the full catalog (#45-#49): language version
managers (mise, asdf, pyenv, uv, Conda, rbenv, RVM, ruby-install, nvm,
rustup), shared dependency/build caches (Cargo home, npm, pnpm,
Gradle, Maven, Go, pip), Apple/Android tooling (Xcode, CoreSimulator,
Android SDK), and model/VM stores (Homebrew, Hugging Face, Ollama,
Docker Desktop's sparse backing file, OrbStack) -- see
`docs/locations.md` for the full table of every detector, its
locations, overrides, categories, and documented limits (e.g. pnpm's
per-volume stores, Maven's undecidable downloaded-vs-local split).
`external_pruned_subtrees` names a detector-resolved location that
folded into one of `roots` as a nested subtree and was pruned from
that root's walk because it is separately measured as its own external
unit (`--view external`) -- the mechanism that keeps a location's
bytes counted exactly once instead of twice (see
`coverage-and-history.md`'s "External/shared storage units").

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
`--offset`. With `--project`, the envelope also gains `agent_storage:
{units, total_bytes}` -- this project's own linked agent-storage units
(see `agent-storage.md`), the same linkage `--view agents --project
NAME` reports, present here too so a project-scoped query never has to
also pass `--view agents` to see it.

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

With no explicit root, the envelope also gains `scope_coverage` (an
array) whenever any root in the configured scope is not cleanly
`complete` this pass -- see `coverage-and-history.md`'s "Multi-root
observation and per-root coverage". An explicit single root never
carries this key: nothing about its scope is ambiguous.

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
| `external` | object: `{units: [{detector_id, detector_name, category, provenance, path, bytes, growth_bytes, regrowth_count, consumers, note}], total_bytes}` | Storage with no containing project (Cargo registry, rustup, Homebrew, ...). `total_bytes` is separate from `reconciliation` above -- never sum the two. Shown for review only, never markable in the TUI and never actionable through any command: removing one means the manager that owns it fetches or rebuilds it again next time. |

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

## There is no propose/execute/plans/grant command

Removed 2026-09-23 ("swamp reports; the human removes"): there is no
plan, no grant, no `swamp propose`/`execute`/`plans`/`grant`. A row's
facts (bytes, growth, recovery cost, warnings) are exactly what
`report`'s views already show; nothing produces a separate "plan"
object, and nothing authorizes or executes anything. The only thing
that moves a path to the Trash is a human in the TUI (Space marks,
Backspace shows current facts, Enter moves it) or at a shell. See
`cleanup-and-recovery.md` for where a TUI move went and how to restore
it, and `trust-model.md` for the full statement of what changed.

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
