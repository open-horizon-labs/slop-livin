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

Every removal (`fs::rename`/`remove_file`/`remove_dir`/`remove_dir_all`, `trash::*`) anywhere in the workspace whose subject is caller-supplied -- a parameter, a field of one, or bindings and iteration over those; not a path the function constructed from a literal, a local path helper or a file it created -- must be preceded by all three live rechecks, honoured, in its body or in a helper called before it (followed transitively), and the removed path must be one the rechecks were about. No whole-file exemptions. Docker removals follow an honoured `docker::still_removable`, and only `docker::remove` builds a destructive `docker` subprocess.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `execution_sinks_recheck_live_state/01-discarded-protection`, `execution_sinks_recheck_live_state/02-aliased-rename`, `execution_sinks_recheck_live_state/03-second-destructive-call`, `execution_sinks_recheck_live_state/04-honoured-sink-passes`, `execution_sinks_recheck_live_state/05-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

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
