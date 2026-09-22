---
id: occupancy-is-tristate-at-sinks
severity: hard
statement: "A sink never consumes a boolean occupancy answer. Occupancy is OccupancyState::{Free, Occupied(path), Unknown(reason)}; a probe that could not run, timed out, or was denied permission is Unknown, and Unknown refuses. Directories are probed so every member is covered, not just the anchor: lsof +D on macOS, a procfs scan of every open file, cwd and mapping on Linux (#86)."
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

- `occupancy.rs` must define `OccupancyState` with `Free`, `Occupied`
  and `Unknown` variants.
- `actions.rs`, `cargo_cleanup.rs` and `crates/tui/src/actions.rs` must
  not call the boolean `occupancy::occupied(` or `agents::is_active(`.
- No empty `Unknown` arm (`Unknown(_) => {}` and its spellings) in those
  files.

**Limits.** The audit rejects the *empty* `Unknown` arm; it cannot tell
a refusal from an arm that merely logs. Statement-level review and the
runtime test cover the rest.

## Runtime tests that complete it

- `crates/core/tests/reviewer_counterexamples.rs::open_cache_member_must_stop_parent_removal`
- `crates/core/tests/execution_rechecks.rs` — an occupancy probe forced
  to `Unknown` refuses and moves nothing.
- `crates/core/src/recheck.rs::tests::member_occupancy_probes_a_descendant_not_only_the_anchor`
