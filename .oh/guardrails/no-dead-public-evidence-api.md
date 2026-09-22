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

Collect every `pub fn` (free functions and inherent impl methods) in the
listed modules, then scan every non-test function body in
`crates/{core,cli,tui}/src` for a call to it. A function whose only
"caller" is its own definition does not count. `#[cfg(test)]` modules
and `tests/` are excluded, so a test-only caller does not rescue a dead
API.

**Limits.** Name-based: two functions with the same name in different
modules are indistinguishable, which can hide a genuinely dead one.
Trait-dispatched and serde-derived entry points are exempted by name.

## Runtime tests that complete it

- the executable matrix↔docs test where a table exists
  (`crates/core/tests/agent_matrix_matches_docs.rs`)
- for prose claims, the session note lists each claim with the test that
  backs it
