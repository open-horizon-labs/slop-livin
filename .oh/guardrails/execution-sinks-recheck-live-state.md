---
id: execution-sinks-recheck-live-state
severity: hard
statement: "Every function that moves or removes user data recomputes three things against live state before its first destructive call: the reviewed identity and membership of the unit (recheck::reviewed_snapshot), human keep/protect intent loaded fresh in both directions (recheck::live_protection), and occupancy over every member, where Unknown is a refusal (recheck::member_occupancy). An approval authorizes exactly what was reviewed, not whatever is at that path now."
outcome: decision-relevant-storage-evidence
audit: gate_paths_only_inside_gates, sinks_have_no_path_predicates
compile_fail:
  - trash_move_needs_a_recheck_proof
  - recheck_proof_is_minted_only_by_run_all
  - recheck_proof_is_not_clone
  - recheck_proof_is_spent_once
runtime_tests:
  - crates/core/tests/reviewer_counterexamples.rs
  - crates/core/tests/execution_rechecks.rs
  - crates/core/src/recheck.rs
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

Mechanism: type, gate audit, clippy, runtime test.

**Type.** Every destructive operation (`fs_gate::destroy::{trash_move, Envelope}`, Docker removal, worktree prune) takes a `RecheckProof`, which only `recheck::run_all` mints (snapshot + live protection both directions + member occupancy, fail closed), which is not `Clone`, is consumed by the call, and is refused when older than `MAX_PROOF_AGE` or when it does not cover the path -- plus an `Authorized` token.

**Gate audit.** `std::fs` removal and rename, `trash` and `libc` exist only in the gate; `fs_gate::destroy` and `recheck::run_all` may be named only by the execution sinks.

**Clippy.** `disallowed_methods` rejects `std::fs::{rename, remove_*}` type-resolved outside the gate.

Retired 2026-09-22: the `execution_sinks_recheck_live_state` source audit (a `syn` call-graph rule, which four review rounds showed cannot be made mutation-proof without type resolution; `docs/architecture.md`, "Capability gates"). Its mutation fixtures, and the sweep-3 and sweep-4 mutations aimed at it, now run in `crates/source-audit/tests/mutation_sweep.rs`, compiled: each must fail compilation (or clippy) or a gate audit.

Compile-fail cases (`crates/core/tests/compile_fail/`, run by `crates/source-audit/tests/compile_fail.rs` against the production API): `trash_move_needs_a_recheck_proof`, `recheck_proof_is_minted_only_by_run_all`, `recheck_proof_is_not_clone`, `recheck_proof_is_spent_once`.

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
