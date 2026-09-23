---
id: occupancy-is-tristate-at-sinks
severity: hard
statement: "A sink never consumes a boolean occupancy answer. Occupancy is OccupancyState::{Free, Occupied(path), Unknown(reason)}; a probe that could not run, timed out, or was denied permission is Unknown, and Unknown refuses. Directories are probed with lsof +D so every member is covered, not just the anchor."
outcome: decision-relevant-storage-evidence
audit: gate_paths_only_inside_gates
compile_fail:
  - occupancy_has_no_bool
  - occupancy_does_not_convert_to_bool
runtime_tests:
  - crates/core/tests/reviewer_counterexamples.rs::open_cache_member_must_stop_parent_removal
  - crates/core/tests/execution_rechecks.rs
  - crates/core/src/recheck.rs::tests::member_occupancy_probes_a_descendant_not_only_the_anchor
---

## Rationale

`open_cache_member_must_stop_parent_removal`: with `debug/log.txt` held
open, removing `debug/` still completed. Two causes, one shape — the
sink asked a boolean question about the anchor path only. A boolean
cannot distinguish "checked, nothing open" from "could not check", and
`lsof -- <dir>` says nothing about the directory's contents.

## Detection

Mechanism: type, gate audit, runtime test.

**Type.** `OccupancyState` (`Free | Occupied | Unknown`) is `#[must_use]`, has no `is_free` and no conversion to `bool`; its one consumer on the destructive path is the private `recheck::member_occupancy`, where `Unknown` is a refusal, reached only through `run_all` (proposal time gets a refusal string from `recheck::occupancy_refusal`). The boolean `is_active` view exists only under the `testing` feature.

**Gate audit.** `gate_paths_only_inside_gates`: `OccupancyState` may be named only in `occupancy` and `recheck`, so no other module can match on it, collapse it or read `Unknown` as free.

Retired 2026-09-22: the `occupancy_is_tristate_at_sinks` source audit (a `syn` call-graph rule, which four review rounds showed cannot be made mutation-proof without type resolution; `docs/architecture.md`, "Capability gates"). Its mutation fixtures, and the sweep-3 and sweep-4 mutations aimed at it, now run in `crates/source-audit/tests/mutation_sweep.rs`, compiled: each must fail compilation (or clippy) or a gate audit.

Compile-fail cases (`crates/core/tests/compile_fail/`, run by `crates/source-audit/tests/compile_fail.rs` against the production API): `occupancy_has_no_bool`, `occupancy_does_not_convert_to_bool`.

## Runtime tests that complete it

- `crates/core/tests/reviewer_counterexamples.rs::open_cache_member_must_stop_parent_removal`
- `crates/core/tests/execution_rechecks.rs` — an occupancy probe forced
  to `Unknown` refuses and moves nothing.
- `crates/core/src/recheck.rs::tests::member_occupancy_probes_a_descendant_not_only_the_anchor`
