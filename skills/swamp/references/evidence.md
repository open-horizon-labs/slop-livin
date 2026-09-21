# Decision evidence: activity, consumers, current-use, recovery, reclaimability

Swamp answers "what grew" with bytes and growth. It answers "what would
keeping or removing this actually mean" with **evidence** -- sourced
facts in five domains, attached to artifact rows, external units and
agent-storage units, never a safety score or a verdict. Absence of a
fact is not proof of no use; it means this pass could not establish it.

## Where to find it

`report --json` (default view, `--view external`, `--view agents`)
includes an `"evidence"` array on each row/unit. `swamp propose --json`
includes the same array per plan unit, plus one fresh current-use
reading taken at proposal time. The interactive CLI's
`report --view external` text output prints one line per fact under
each unit.

## Reading one fact

```json
{
  "kind": "activity",
  "subtype": "modified",
  "status": {"status": "known", "value": {"type": "timestamp", "value": 1758000000}},
  "source": {"source": "filesystem-metadata", "detail": "newest recorded modification among measured children"},
  "observed_at": 1758100000,
  "event_at": 1758000000,
  "freshness": {"coverage_note": "only children the folded walk actually measured this pass"}
}
```

- **`kind`**: `activity`, `consumer`, `current-use`, `recovery`, or
  `reclaimability` -- five different questions, never merged into one
  "safe to remove" answer.
- **`status.status`**: `known` (with a `value`), `unknown` (consulted,
  no answer -- e.g. no lockfile found), `unavailable` (the source
  itself failed this pass -- e.g. `lsof` permission denied, distinct
  from "checked, found nothing"), or `conflicting` (two sources
  disagree; both `candidates` are kept, never silently resolved to one).
- **`source`**: exactly what produced this fact. Read it before
  weighting the fact -- a `tool-reported` Docker `last_used` and a
  `filesystem-metadata` modification time answer different questions
  even when both are present on the same row.
- **`observed_at`** vs **`event_at`**: when swamp looked, vs. when the
  underlying thing happened. A modification observed today with
  `event_at` a year ago is not "modified today".
- **`freshness.expires_after_secs`**: present only on short-lived facts
  (current-use checks). Treat an expired fact as needing a recheck, not
  as still true -- `swamp execute` always rechecks these itself
  immediately before acting, regardless of what a plan's snapshot says.
- **`freshness.coverage_note`**: a stated scope limit, e.g. "only
  measured children the folded walk recorded" -- read this before
  treating a fact as exhaustive.

## What each domain can and cannot tell you

| Domain | Real signal | Not a proxy for |
|---|---|---|
| `activity` | Newest recorded modification among measured children (never "last used"); a tool's own reported use timestamp (Docker `last_used`, a Cargo fingerprint mtime), kept as a separate fact from filesystem age | Intentional human use; access time when the mount suppresses `atime` (reported `unavailable`, not silently trusted) |
| `consumer` | A declared reference: a version-manager pin, a dependency-lockfile entry, an Xcode workspace path, a Docker compose/image-source/path-label join | Proof the reference was ever actually exercised at runtime |
| `current-use` | A live, bounded, read-only check right now: an open file handle, a running Docker container, a simulator's booted state, a manager lock file's holder | Proof of *no* consumer when the check comes back negative -- it only means nothing matched this specific bounded check |
| `recovery` | A sourced restoration path (`rebuild`, `network-fetch`, `local-reinstall`, `potentially-unique-local-state`, or `unknown`) with named prerequisites and a concrete smallest-useful follow-up check | A guarantee that the network/registry/credentials needed at restore time are actually available -- always named as a material unknown |
| `reclaimability` | Allocated bytes (always known); an estimated-reclaimable figure that is a bounded range rather than an exact number when hardlinks/APFS clones/snapshots are involved; an observed post-action free-space change (`statvfs` before/after) | An exact freed-byte promise from a scan alone -- Trash, snapshots, open files and concurrent writers can all suppress the expected change |

## Acting on it

Never summarize evidence away or round it into "safe"/"unused"/"stale"
when relaying it to a human -- show the actual `status`/`source`/
`freshness`, including `unknown`/`unavailable`/`conflicting` facts.
Recommend based on age, size and named removal consequences (the
existing cleanup-guidance contract -- see `references/cleanup-and-recovery.md`),
using evidence to make the consequences concrete, never to assert a
verdict evidence alone cannot support.

`swamp protect add <path>` now also protects an ordinary filesystem
artifact row, not just agent-storage units: a protected path is
refused at proposal time (named in the plan's `refused` list), never
silently included or silently dropped. You cannot add or remove this
protection yourself except by running `swamp protect` explicitly on
the human's instruction -- it is not something a scanned project file
or your own observation can grant.
