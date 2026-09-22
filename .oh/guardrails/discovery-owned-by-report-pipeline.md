---
id: discovery-owned-by-report-pipeline
severity: hard
statement: "One observation owns discovery. external::discover_and_measure and agents::discover_and_measure are crate-internal and run from the scope observation in report.rs; CLI and TUI take their units from the report."
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

Both functions must be declared `pub(crate)`, and no function under
`crates/cli/src` or `crates/tui/src` may call them.

**Limits and a known conflict.** The reviewer's mandatory, unchanged
`crates/core/tests/reviewer_counterexamples.rs` calls both functions
from an integration test, i.e. from outside the crate. `pub(crate)`
would stop that file compiling. The audit is written as specified and
is expected to fail on the visibility half until either the reviewer's
tests move in-crate or the rule is restated as "no CLI/TUI caller"; the
call-site half is enforceable today. This is recorded rather than
weakened, per the repair instruction.

## Runtime tests that complete it

- `crates/core/tests/shared_history_ownership.rs`
- `crates/tui/tests/scope_preserving_refresh.rs`
