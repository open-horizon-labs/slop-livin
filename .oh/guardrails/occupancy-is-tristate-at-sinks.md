---
id: occupancy-is-tristate-at-sinks
severity: hard
statement: "A sink never consumes a boolean occupancy answer. Occupancy is OccupancyState::{Free, Occupied(path), Unknown(reason)}; a probe that could not run, timed out, or was denied permission is Unknown, and Unknown refuses. Directories are probed with lsof +D so every member is covered, not just the anchor."
outcome: decision-relevant-storage-evidence
audit: occupancy_is_tristate_at_sinks
---

## Rationale

`open_cache_member_must_stop_parent_removal`: with `debug/log.txt` held
open, removing `debug/` still completed. Two causes, one shape — the
sink asked a boolean question about the anchor path only. A boolean
cannot distinguish "checked, nothing open" from "could not check", and
`lsof -- <dir>` says nothing about the directory's contents.

## Detection

`OccupancyState` has `Free`, `Occupied` and `Unknown`; no function consumes a boolean occupancy answer (derived: `bool` functions over the tri-state or the probe); no `member_occupancy` answer is discarded; every arm covering `Unknown` refuses; no `matches!`, `==`/`!=` or `if let` that names `Occupied` without `Unknown` collapses the tri-state.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `occupancy_is_tristate_at_sinks/01-discarded-occupancy`, `occupancy_is_tristate_at_sinks/02-boolean-at-a-sink`, `occupancy_is_tristate_at_sinks/03-unknown-falls-through`, `occupancy_is_tristate_at_sinks/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## Runtime tests that complete it

- `crates/core/tests/reviewer_counterexamples.rs::open_cache_member_must_stop_parent_removal`
- `crates/core/tests/execution_rechecks.rs` — an occupancy probe forced
  to `Unknown` refuses and moves nothing.
- `crates/core/src/recheck.rs::tests::member_occupancy_probes_a_descendant_not_only_the_anchor`
