---
id: coverage-changes-are-not-storage-changes
severity: hard
statement: "Changed scan coverage, incomplete observation, or lost access must not be reported as observed filesystem growth or deletion; overlapping coverage must not double-count measured bytes."
outcome: disk-growth-by-project
s_and_t_step: W1b
parent_step: W1
sufficiency_group: G1
owner: null
review_trigger: "A new detector, scope change, contradictory fact, or real user decision exposes a failure."
tactic_disposition: selected
---

# Coverage changes are not storage changes

## Rationale

Preserve accounting and history across coverage changes so users can trust what
actually changed. Unobserved is not deleted. Newly discovered is not proof of
newly created. Removing an include, adding an exclusion, losing permissions, or
disconnecting a volume changes what is known, not necessarily what exists.

This constrains [coverage-aware storage history](../outcomes/coverage-aware-storage-history.md)
and the parent outcome. Shared/project relationship views must not inflate aggregate
totals, and measured bytes must not silently become an exact reclamation promise.

## Selected tactic and parallel assumption

Establish invariants for overlaps, partial scans, identity, and changing roots.
These distinctions are necessary; a particular persistence schema is not selected.
Deterministic fixtures can exercise the observation sequences before broad defaults
are enabled. That is a validation hypothesis, not a claim of completed checks.

## Validation gap

No executable validation is declared for this complete contract. Check nested and
aliased roots, root-order changes, scope additions/removals, permission failures,
disconnected volumes, partial runs, and identity changes against actual filesystem
changes. Preserve the distinction between incomplete observations and tombstones.
Concurrent mutation and shared physical-storage accounting require explicit limits.

The existing separate-root loop and volume-keyed state are evidence of a boundary
that needs checking, not a passed multi-root correctness test. Relevant source:
[observation loop](../../crates/cli/src/schedule.rs) and
[growth/history](../../crates/core/src/growth.rs).

## Lineage and invalidation

Source: 2026-09-19 problem weave, W1b, parent W1, depth 2, G1 (W1a + W1b, all
required). Technical framing; proposed contributing role is core engineering,
with no owner assigned. Recording the hard constraint did not select its tactic;
the subsequent [solution-space session](../sessions/2026-09-19-developer-storage-coverage-and-evidence.md)
does so without asserting implementation. False disappearance, inflated totals, or root-order-
dependent history invalidates the implementation against this constraint.
