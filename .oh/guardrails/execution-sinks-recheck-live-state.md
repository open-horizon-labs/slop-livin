---
id: execution-sinks-recheck-live-state
severity: hard
statement: "Every function that moves or removes user data recomputes three things against live state before its first destructive call: the reviewed identity and membership of the unit (recheck::reviewed_snapshot), human keep/protect intent loaded fresh in both directions (recheck::live_protection), and occupancy over every member, where Unknown is a refusal (recheck::member_occupancy). An approval authorizes exactly what was reviewed, not whatever is at that path now."
outcome: decision-relevant-storage-evidence
audit: execution_sinks_recheck_live_state
---

## Rationale

The 2026-09-21 independent review reproduced three separate ways an
approved plan spent its authorization on data nobody had reviewed:

- `replacement_directory_must_not_spend_old_approval` — the reviewed
  `debug/` directory was renamed aside and a new `debug/` created with
  unrelated content. Execution checked only `is_dir()`, so the
  replacement was moved to Trash under the old approval.
- `protection_added_after_approval_must_stop_execution` — a `swamp
  protect` entry added after approval was never consulted again, so the
  protected data was moved.
- `open_cache_member_must_stop_parent_removal` — occupancy was probed on
  the unit's anchor path only, so a file held open *inside* the cache
  directory did not stop the parent's removal.

These were three symptoms of one missing thing: a sink that re-derives
live state. `crate::recheck` is that one model, and this audit is what
keeps a fourth sink from being written without it.

## Detection

For every function in `actions.rs`, `cargo_cleanup.rs` and `docker.rs`
whose body contains a destructive call (`fs::rename`, `remove_file`,
`remove_dir`, `remove_dir_all`, `cargo_cleanup::move_reviewed`,
`docker::remove`), the audit takes the token text of the body up to the
first destructive call and requires all three recheck calls to appear in
it — either directly, or inside a helper that is itself called in that
prefix (the call graph is followed transitively across
`crates/{core,cli,tui}/src`).

**Limits.** This is statement-order within one token stream, so it
proves "the recheck code runs before the destructive code on this path",
not "the recheck's *result* is honoured". That second half is what the
runtime tests are for. It also cannot see a destructive call made
through a dynamically dispatched trait object.

## Runtime tests that complete it

- `crates/core/tests/reviewer_counterexamples.rs` — the three review
  counterexamples above, unchanged.
- `crates/core/tests/execution_rechecks.rs` — protection added on an
  ancestor and on a descendant after approval, a member appended after
  approval, a replaced directory, an open member, occupancy `Unknown`,
  and a corrupt protect file: each refuses, moves nothing, and records
  the refusal in the ledger.
- `crates/core/src/recheck.rs` unit tests — drift detection for the
  exact and summarized membership modes.
