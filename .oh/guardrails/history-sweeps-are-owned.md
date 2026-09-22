---
id: history-sweeps-are-owned
severity: hard
statement: "A sweep of the shared current table may mark a row absent only when the row belongs to the observation's own key family and lies inside a region that observation covered completely this pass. Exclusion, a disabled detector, a missing or unreadable root, or simply not being part of this pass all mean the row is left alone. Coverage changes are not storage changes."
outcome: coverage-aware-storage-history
audit: history_sweeps_are_owned
---

## Rationale

`unchanged_combined_observation_must_not_invent_regrowth`: external and
agent discovery share one current table (deliberately — one store, one
key family, two granularities), and each swept it for keys it had not
seen. Running them in sequence on a completely unchanged filesystem
therefore tombstoned the other family's rows, and the next pass recorded
the resurrection as regrowth. The TUI's startup calls them in exactly
that order.

Invented growth is the failure mode that destroys trust fastest: the
user sees a number move on a disk where nothing moved.

## Detection

- `growth.rs` must define `ObservationOwnership` (family +
  covered_roots), and `observe_and_annotate_external` must take it.
- The statement that sets `present = false` must be preceded in the
  token stream by an `ownership.owns(..)` (or `ownership.covers(..)`)
  guard.
- No function anywhere in `crates/{core,cli,tui}/src` may construct a
  wildcard ownership (`ObservationOwnership::all()`/`::wildcard()`).

**Limits.** The audit proves a guard exists before the tombstone, not
that `owns` is implemented correctly; the runtime tests carry that.

## Runtime tests that complete it

- `crates/core/tests/reviewer_counterexamples.rs::unchanged_combined_observation_must_not_invent_regrowth`
- `crates/core/tests/shared_history_ownership.rs` — external-then-agent
  and agent-then-external both yield zero regrowth and byte-identical
  stored rows; disabling a detector between observations yields coverage
  notes and no tombstones; re-enabling yields no regrowth; a real delete
  and recreate yields exactly one regrowth.
