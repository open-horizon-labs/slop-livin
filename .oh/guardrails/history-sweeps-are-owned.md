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

Every tombstone anywhere (see `coverage_changes_are_not_storage_changes` for the recognition) must be inside an ownership condition; a function that sweeps under an `ObservationOwnership` verdict must be handed the window as a parameter; no wildcard window is constructed.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `history_sweeps_are_owned/01-discarded-ownership`, `history_sweeps_are_owned/02-guard-not-the-condition`, `history_sweeps_are_owned/03-wildcard-ownership`, `history_sweeps_are_owned/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## Runtime tests that complete it

- `crates/core/tests/reviewer_counterexamples.rs::unchanged_combined_observation_must_not_invent_regrowth`
- `crates/core/tests/shared_history_ownership.rs` — external-then-agent
  and agent-then-external both yield zero regrowth and byte-identical
  stored rows; disabling a detector between observations yields coverage
  notes and no tombstones; re-enabling yields no regrowth; a real delete
  and recreate yields exactly one regrowth.
