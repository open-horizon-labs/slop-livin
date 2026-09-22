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
counterexamples carry that. The 2026-09-22 re-review's CE1 is what that
limit cost: `authorized_detector_paths_in_explicit_roots` tested
exclusion only against roots whose *status* was `Excluded`, and under
`--root <parent>` an excluded home is not a root at all but a
`PruneNote` inside the explicit root — so `swamp report --root ~` offered
an excluded agent home's contents for removal. Both entry points now
route every exclusion decision through the one
`EffectiveScope::exclusion_for` helper, so explicit and configured scope
cannot diverge again.

## Runtime tests that complete it

- `crates/core/tests/reviewer_counterexamples.rs::excluded_agent_home_must_not_be_scanned`
- `crates/core/src/scope.rs::tests::authorized_roots_drops_excluded_and_explains_missing`
- `crates/core/src/scope.rs::tests::explicit_roots_do_not_authorize_detector_paths_outside_them`
- `crates/core/tests/reviewer_counterexamples_stack2.rs::an_excluded_home_must_stay_excluded_under_an_explicit_root`
  — the CE1 case the prior test could not see, because it passed `&[]`
  for explicit roots.
- `crates/core/tests/explicit_root_scope_exclusions.rs` — every
  exclusion case (an excluded home, an excluded nested location, a
  disabled detector, a protected path in either spelling) under an
  explicit root that *contains* it.
- `crates/cli/tests/` — `report <root> --view agents --json` on a
  fixture root that does not contain the agent home returns zero units
  with a coverage note saying why.
