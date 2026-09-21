# Coverage, history, and what a growth number proves

Swamp keeps current-state + reverse-delta history per volume under
`$SWAMP_DIR`. A growth number is only ever a comparison between two
real observations -- never a projection, and never a fact conjured from
a single snapshot.

## `since` resolution

Every command that reports growth resolves `since` in this order: the
explicit `--since` you passed, else `config.toml`'s configured
`since`, else the hard-coded default (`24h`). `report --json`'s
envelope always echoes the *effective* value back as `"since"` --
never assume your raw argument is what was actually used; read the
field.

## History span vs. asked window

`--view grown --json`'s `coverage.history` block:

```json
{
  "history_secs": 0, "asked_window_secs": 3600, "effective_window_secs": 0,
  "note": "asked for 3600s of growth but the store holds 0s of observations; growth is reported over 0s"
}
```

- `history_secs`: how much history this store actually holds for this
  root's volume (`null` if there is none at all).
- `asked_window_secs`: your `--since`, parsed.
- `effective_window_secs`: the window growth was *actually* computed
  over -- the shorter of the two.
- `note`: set whenever `asked_window_secs > history_secs` (you asked
  for more than the store can honor) or when there is no history yet
  ("no observations yet: growth cannot be reported"). Always check this
  before quoting a growth figure to a human as if it covered the window
  they expect.

A brand-new store's very first observation has `history_secs: 0` --
this is correct, not a bug: there is nothing yet to diff against. A
"no growth" result under these conditions describes an absence of
comparison, not an absence of change.

## Coverage changes are not storage changes

Re-observing an unchanged filesystem, restarting swamp, enriching
GitHub/Docker facts after the fact, or simply letting time pass never
generates a byte-history delta or a tombstone on its own. If a number
changes between two calls with nothing on disk actually different,
that is a defect to investigate, not an expected refresh artifact.

This also covers *scope* changes: adding a root to `config.toml`'s
`[scan]` table, a detector newly resolving a location, or excluding a
path are changes in what swamp *looks at*, never a change in what
exists on disk. `report`/`observe` persist the resolved scope
(`scope.json` under `$SWAMP_DIR`) and print a one-line note on stderr
when it differs from the last one:

```
coverage changed since last observation: +root /Users/you/.cargo (detector cargo-home), -root /Users/you/old-project (excluded)
```

`swamp scope --json` shows the full resolved scope on demand (roots,
statuses, reasons, the detector catalog, and its version) without
needing to diff two observations yourself -- see
`commands-and-json.md`'s `swamp scope` section. A root omitted from a
`report`/`observe`/`ui`/`schedule` call resolves this same scope, one
shared code path for every command; never assume an agent's or
another command's idea of "the roots" without checking `swamp scope`.

## Reconciliation and unknowns

`--view reconciliation --json`: `{attributed, unowned, walked_total,
du_total, docker_attributed, docker_unowned}`. `du_total` is `null`
unless `--verify-du` (CLI-only; slow, runs a real `du -skPx`) was used.
`walked_total` not matching `attributed + unowned` closely is itself
useful evidence, not a failure to hide -- it usually means permission
denials or an in-progress walk.

`--view unowned --json` rows carry an explicit `reason`
(`outside-any-checkout`, `owned-by-nothing`, `inconclusive-evidence`,
`no-containing-repo`, `shared-cache`, `permission-denied`,
`docker-no-join`) -- never attribute an unowned row to a project by
name similarity yourself; that's exactly what `--view docker`'s
explicit `"unowned, name-alike"` labelling exists to prevent you from
doing silently.

## Filesystem vs. Docker accounting

Filesystem and Docker sizes are separate accounting domains
(`reconciliation.docker_attributed`/`docker_unowned` vs. the
filesystem totals) and should not be summed to predict how much
physical disk space an action will actually reclaim -- hardlinks,
shared image layers, and Docker's own storage driver mean the
relationship is not additive.
