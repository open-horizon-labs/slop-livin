---
id: detector-ids-only-in-registry
severity: hard
statement: "Detector identity lives in locations/ (and the scope layer that consumes it). Consumers -- association wiring, recovery hints, reports, CLI, TUI -- match on capabilities the detector declares (Detector::manager_conventions(), Detector::recovery_hint()), never on detector id constants or id string literals."
outcome: coverage-aware-storage-history
audit: detector_ids_only_in_registry
---

## Rationale

`consumer_wiring.rs` matched manager names onto detector ids by hand, so
adding a detector meant editing a table somewhere else or silently
getting no associations. The same pattern -- knowledge about a detector
held outside the detector -- is what let discovery read raw detector
output instead of the authorized scope, and what makes the "add a
detector, edit four files" failure mode of the adapter layer.

A detector that declares what it satisfies (`pyenv` ↔ `.python-version`)
can be added in one place.

## Detection

No function in `consumer_wiring.rs`, `recovery.rs`,
`external_associations.rs`, `report.rs`, `crates/cli/src/**` or
`crates/tui/src/**` may reference a `*_DETECTOR_ID` constant. The
`Detector` trait must declare `manager_conventions()`.

**Limits.** Bare id *string literals* ("cargo-home") are not caught by
the constant check; the capability requirement is what removes the need
for them.

## Runtime tests that complete it

- `crates/core/tests/external_units.rs` and the association tests, which
  assert a new detector's conventions produce associations without a
  wiring edit
