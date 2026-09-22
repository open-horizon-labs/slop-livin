---
id: no-dead-public-evidence-api
severity: hard
statement: "Every pub fn in the evidence, activity, occupancy, recovery, reclaimability, consumer-wiring, association, toolchain-declaration and recheck modules has at least one non-test caller in the workspace. A capability the docs claim is either wired into the live pipeline or deleted along with the claim."
outcome: decision-relevant-storage-evidence
audit: no_dead_public_evidence_api
---

## Rationale

The PR #123 review's central finding: three of the five #55 current-use
sources, the whole #54 access-time path, and four of seven #59
accounting functions had **zero production callers**, while the
CHANGELOG, `DESIGN.md` and `docs/architecture.md` stated those
capabilities as delivered. A module full of well-tested functions nobody
calls is not a delivered feature; it is a claim with unit tests attached.

Named dead entry points at the time of the review:
`activity::{access_time_evidence, tool_reported_use_evidence,
docker_last_used_evidence}`,
`occupancy::{docker_running_container_evidence, manager_lock_evidence,
simulator_booted_evidence}`,
`reclaimability::{apfs_clone_or_snapshot_bound, sparse_file_accounting,
estimate_selection, observed_free_space_change}`,
`recovery::{maven_artifact_recovery, mutable_environment_recovery,
toolchain_installation_recovery}`.

## Detection

The evidence API is derived: the modules (and child modules) whose public functions return the evidence vocabulary or whose code builds `Evidence`. Every public function and constant there must be reachable from a binary: every CLI `main`, every trait method, and the TUI's public run functions. A caller that is itself unreachable does not count.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `no_dead_public_evidence_api/01-dead-code-allowed-caller`, `no_dead_public_evidence_api/02-dead-pub-fn`, `no_dead_public_evidence_api/03-dead-pub-const`, `no_dead_public_evidence_api/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## Runtime tests that complete it

- the executable matrix↔docs test where a table exists
  (`crates/core/tests/agent_matrix_matches_docs.rs`)
- for prose claims, the session note lists each claim with the test that
  backs it
