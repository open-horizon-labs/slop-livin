---
id: coverage-aware-storage-history
kind: capability
status: proposed
outcome: disk-growth-by-project
s_and_t_step: W1
parent_step: W0
sufficiency_group: G0
owner: null
review_trigger: "A new detector, scope change, contradictory fact, or real user decision exposes a failure."
tactic_disposition: selected
files: []
---

# Coverage-aware storage history

## Statement

Developers can tell what swamp measured, what it did not measure, and what actually
changed within that coverage, including storage outside project checkouts.

## Why it matters

Ambiguous totals or changes undermine the [parent outcome](disk-growth-by-project.md).
Newly observed, excluded, unreadable, or disconnected storage must not manufacture
growth or disappearance. Overlapping roots must not double-count measured bytes.

## Enables

Useful growth reporting across a visible scan boundary, even before every artifact
has an established project association. It enables honest comparisons rather than
a claim that the whole disk has been accounted for.

## Selected tactic and justification

Define the meaning and limits of each reported total and change. This semantic
clarity is necessary; no particular schema or configuration syntax is established
as indispensable. Existing observation history provides a foundation, but the
current separate-root observation loop with volume-keyed state leaves multi-root
correctness unresolved. This record does not claim that capability is implemented.

## Acceptance signal

Representative overlapping-root, added-root, excluded-root, partial-scan, and
unreadable-root cases distinguish actual filesystem changes from changed coverage.
Reordering roots must not change the interpretation of the same observation.
Users can explain a total's scope without inspecting the implementation.
Executable validation for this full contract has not yet been established.

## Lineage and collective sufficiency

Source: 2026-09-19 problem weave, W1, parent W0, depth 1; primarily technical
framing with the user's explicit coverage-control preference.

**G1**, parent W1, children **W1a + W1b**, all required: understandable/controllable
coverage and sound accounting/history jointly support this capability. Concurrent
filesystem changes remain a validation gap.

- W1a: inspectable and controllable effective coverage; selected in the
  [full-scope session](../sessions/2026-09-19-developer-storage-coverage-and-evidence.md),
  not separately recorded. Built-in detection plus config differences and visible
  upgrade scope changes are selected; concrete implementation syntax remains bounded
  by the session's precedence and coverage requirements.
- W1b: [coverage changes are not storage changes](../guardrails/coverage-changes-are-not-storage-changes.md).

Proposed contributing roles are product/workflow for coverage controls and core
engineering for accounting/history; ownership is unassigned. Invalidate this
framing if correct bounded measurements do not resolve decision uncertainty, or
if ordinary correctness requires unaffordable observation work.
