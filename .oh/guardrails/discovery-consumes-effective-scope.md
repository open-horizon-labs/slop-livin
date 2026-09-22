---
id: discovery-consumes-effective-scope
severity: hard
statement: "Only crate::scope interprets detector candidates. Discovery and measurement consume EffectiveScope::authorized_roots(), which has already applied exclusions, disabled detectors and explicit-root replacement. An excluded home yields zero units; a disabled detector yields zero units; a missing or unreadable root yields a coverage note and zero units, never a tombstone."
outcome: coverage-aware-storage-history
audit: discovery_consumes_effective_scope
---

## Rationale

`excluded_agent_home_must_not_be_scanned` in the 2026-09-21 review:
excluding the entire Claude Code home still produced one agent unit and
one external unit. Both `external::discover_and_measure` and
`agents::discover_and_measure` iterated `scope.detectors`' raw
`Resolved` candidates, which is detector *output*, not authorized
scope — so no exclusion, no disabled detector and no explicit-root
replacement could reach them. The CLI and TUI compounded it by
re-resolving the scope with empty explicit roots, widening commands the
user had deliberately narrowed.

An exclusion the tool accepts and then ignores is worse than an
exclusion it refuses.

## Detection

- No function in `external.rs` or `crates/core/src/agents/**` may
  reference `LocationStatus::Resolved`, `scope.detectors` or
  `summary.locations`.
- `scope.rs` must define `EffectiveScope::authorized_roots()`, and both
  `external.rs` and `agents/mod.rs` must call it (or its explicit-root
  companion).

**Limits.** The audit proves the *seam* is used, not that the seam's
semantics are right; `scope.rs`'s own tests and the reviewer
counterexample carry that.

## Runtime tests that complete it

- `crates/core/tests/reviewer_counterexamples.rs::excluded_agent_home_must_not_be_scanned`
- `crates/core/src/scope.rs::tests::authorized_roots_drops_excluded_and_explains_missing`
- `crates/core/src/scope.rs::tests::explicit_roots_do_not_authorize_detector_paths_outside_them`
- `crates/cli/tests/` — `report <root> --view agents --json` on a
  fixture root that does not contain the agent home returns zero units
  with a coverage note saying why.
