---
id: discovery-owned-by-report-pipeline
severity: hard
statement: "One observation owns discovery. Outside tests, only report.rs::observe_scope may run external::discover_and_measure or agents::discover_and_measure, and it must run both; CLI and TUI take their units from that observation."
outcome: coverage-aware-storage-history
audit: discovery_owned_by_report_pipeline
---

## Rationale

Ordering only started to matter because there were several independent
passes over one shared history table: the CLI ran its own, the TUI ran
its own at startup, and `report` ran a third. That is how
`unchanged_combined_observation_must_not_invent_regrowth` became
possible at all, and why the TUI's agent/external views could be stale
while its header said "live". `ObservationOwnership` makes each sweep
safe; a single pass makes the question not arise.

## Detection

Three checks, all on non-test code:

1. Both `discover_and_measure` functions still exist.
2. `report.rs::observe_scope` calls **both** of them. Splitting them
   back into separate observations is how their ownership windows could
   disagree again, so one owner that runs only one pass fails.
3. No other function in `crates/core/src`, `crates/cli/src` or
   `crates/tui/src` calls either.

## Why this checks call sites and not visibility

As first written, this guardrail required both functions to be
`pub(crate)`. That is directly incompatible with the reviewers' own
mandatory `crates/core/tests/reviewer_counterexamples.rs`, which calls
both from an integration test -- from outside the crate. That file is
copied in byte-for-byte and may never be edited, so `pub(crate)` would
stop the required evidence compiling. The rule and the evidence could
not both be satisfied, and the evidence wins.

Visibility was never the property the review falsified. What it found
was two *passes* over one table in an order nobody declared. So the rule
is now that, precisely. **Inside the crate this is strictly stronger
than the visibility check it replaces**: `pub(crate)` permitted any
number of core-internal passes, and the call-site rule permits one.
Integration tests may still call either function directly, which is how
the counterexamples exercise them in isolation.

Decision taken 2026-09-21 by the integration owner, recorded here rather
than left as a failing audit with a footnote.

## Limits

- The audit reads call *syntax*, so a call reached through a function
  pointer or a trait object would not be seen. Nothing in this workspace
  does that, and the two functions are concrete free functions.
- It says nothing about how often `observe_scope` itself is called. A
  caller that calls it twice for the same scope in one command would
  pass this audit; `crates/core/tests/shared_history_ownership.rs` is
  what makes that harmless, because each pass carries its own
  `ObservationOwnership`.
- `ObservationParts` lets a caller ask for fewer parts. A part not
  observed is a part not swept, so that is a cost decision rather than a
  correctness one -- but it does mean "this row is absent" is only ever
  concluded from a pass that actually covered it.

## Runtime tests that complete it

- `crates/core/tests/shared_history_ownership.rs` -- both orders, zero
  fabricated regrowth; a disabled detector between observations produces
  no tombstones.
- `crates/tui/tests/scope_preserving_refresh.rs` -- the TUI's refreshes
  carry external and agent units from the same observation.
- `crates/core/tests/store_contents_are_allowlisted.rs` -- drives
  `observe_scope` end to end.
