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
audit: coverage_changes_are_not_storage_changes
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

## Detection

Every tombstone (`.present = ` a value that is not provably `true`, including through a `&mut` binding) and every regrowth bump (`.regrowth_count += n`, `= .. + n`, or a binding computed that way) anywhere must sit inside a condition that makes the row this observation's: an `ObservationOwnership` verdict method, a negated membership test on a region the caller declared unconfirmed (a parameter), or, for a regrowth, a keyed lookup of an observed row. `ObservationOwnership` has `excluded_subtrees` and `covers` reads it.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `coverage_changes_are_not_storage_changes/01-unguarded-tombstone`, `coverage_changes_are_not_storage_changes/02-discarded-ownership-answer`, `coverage_changes_are_not_storage_changes/03-aliased-tombstone-helper`, `coverage_changes_are_not_storage_changes/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## Runtime tests that complete it

`crates/core/tests/coverage_changes_are_not_storage_changes.rs` — three
passes over an unchanged tree, with the coverage changing between them
(add an exclusion, disable a detector, make a location inaccessible,
switch to an explicit root) produce zero growth, zero regrowth and zero
tombstones. Run by name from `scripts/check.sh`. Plus
`crates/core/tests/reviewer_counterexamples_stack2.rs::a_config_only_exclusion_must_not_invent_growth_or_regrowth`,
the 2026-09-22 re-review's CE4: excluding a nested Cargo location reported
the parent as having grown 64 KB and un-excluding it scored a regrowth,
with zero bytes changed on disk.

## Validation gap

The remaining gap after 2026-09-22 is concurrent mutation during a pass
and shared physical-storage accounting across volumes; neither has an
executable check. Check nested and
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
