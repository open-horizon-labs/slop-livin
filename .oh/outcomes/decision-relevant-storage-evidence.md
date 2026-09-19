---
id: decision-relevant-storage-evidence
kind: capability
status: proposed
outcome: disk-growth-by-project
s_and_t_step: W2
parent_step: W0
sufficiency_group: G0
owner: null
review_trigger: "A new detector, scope change, contradictory fact, or real user decision exposes a failure."
tactic_disposition: selected
files: []
---

# Decision-relevant storage evidence

## Statement

Developers can understand the likely consequences and material unknowns of keeping
or removing an artifact, including shared caches and installed toolchains, without
reconstructing all context outside swamp.

## Why it matters

Inventory and size alone do not establish consequences. Consumer relationships,
observed activity, current use, and recovery conditions answer different questions.
They support the [parent outcome](disk-growth-by-project.md) without replacing the
user's judgment or authorizing removal.

## Enables

Justified keep / remove / investigate decisions and focused follow-up checks.
Unknown ownership does not prevent measurement; partial evidence need not prevent
every decision. Whole-machine monitoring is not a prerequisite established here.

## Selected tactic and justification

Organize evidence around decision questions. Relevant context is necessary, but
neither a universal last-used field nor a particular presentation is indispensable.
Existing project context provides part of the foundation. The hypothesis that
additional evidence reduces investigation remains untested on representative cases.

## Acceptance signal

In real decision episodes, users distinguish observations from inference, identify
affected work and recovery prerequisites where known, and name the smallest useful
check for a consequential unknown. Outside checks decrease without an increase in
unsupported confidence. No benchmark or numeric threshold is established yet.

## Lineage and collective sufficiency

Source: 2026-09-19 problem weave, W2, parent W0, depth 1; user/workflow and
evidence/trust passes. Proposed contributing role: domain/evidence work; owner
remains unassigned.

**G2**, parent W2, children **W2a + W2b**, all required: bounded consumer/activity
claims plus recovery conditions and actionable unknowns jointly explain
consequences. Complete knowledge of future needs is outside the coverage claim.

- W2a: [activity and consumer evidence have limits](../guardrails/activity-and-consumer-evidence-have-limits.md).
- W2b: explain restoration prerequisites and the smallest useful follow-up check;
  selected in the [full-scope session](../sessions/2026-09-19-developer-storage-coverage-and-evidence.md),
  not separately recorded by this request.

Invalidate the mechanism if additional fields add reading without changing decisions,
users equate inactivity with expendability, or supposed recovery cannot restore
required state under its disclosed conditions. Universal execution tracking remains
deferred pending evidence that it is necessary.
