---
id: activity-and-consumer-evidence-have-limits
severity: hard
statement: "Consumer, activity, and current-use claims must retain their source, freshness, and limits; missing evidence must not be presented as unused or safe to remove."
outcome: disk-growth-by-project
s_and_t_step: W2a
parent_step: W2
sufficiency_group: G2
owner: null
review_trigger: "A new detector, scope change, contradictory fact, or real user decision exposes a failure."
tactic_disposition: selected
audit: activity_and_consumer_evidence_have_limits
---

# Activity and consumer evidence have limits

## Rationale

Separate consumers, activity, and current-use evidence to support
[decision-relevant storage evidence](../outcomes/decision-relevant-storage-evidence.md).
Referenced, accessed, modified, and running establish different facts. A filesystem
timestamp alone does not identify a developer's last intentional use. No known
configuration reference does not prove that a tool version is unused. Shared
artifacts may have multiple consumers, including consumers outside scanned roots.

## Selected tactic and parallel assumption

Preserve source, freshness, coverage, and interpretive limits for each claim.
This discipline is necessary; a universal activity collector or confidence score
is not selected. Ecosystem-specific evidence may narrow uncertainty without
whole-machine execution monitoring. Its decision value still needs testing.

## Boundaries

- Distinguish direct observations, declared relationships, and inferred associations.
- Distinguish a fact's event time from the time swamp observed it.
- Unknown remains unknown; it does not become a safety verdict or authorize removal.
- A missing tool executable does not prove its stored installations are absent.
- Keep measurement useful even when attribution is incomplete.
- Explain consequential unknowns without requiring every unknown to block a decision.

This elaborates the existing
[facts-not-verdicts guardrail](agent-interface-facts-not-verdicts.md); its existing
vocabulary audit does not establish provenance, freshness, or semantic correctness.

## Detection

A `FactStatus` variant is built only in the module that declares it (struct literals anywhere else, including inside macro arguments, fail). The reasoned constructors are derived: every `Evidence` method with a `reason` parameter; the argument in that position must not evaluate empty -- a literal `""`, a constant holding `""` (constants are followed), `String::new()` or `Default::default()`. Every evidence builder in the activity module names an `EvidenceSource` or delegates to one that does, and `render::render_evidence_lines` prints the reason.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `activity_and_consumer_evidence_have_limits/01-status-struct-literal`, `activity_and_consumer_evidence_have_limits/02-empty-reason`, `activity_and_consumer_evidence_have_limits/03-activity-evidence-without-source`, `activity_and_consumer_evidence_have_limits/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## Runtime tests that complete it

- `crates/core/tests/evidence_contract.rs` — the rendered form of an
  `Unknown`/`Unavailable` fact contains its reason, and the docs table
  lists every `ACTIVITY_EVIDENCE_INVENTORY` entry.

## Validation gap

What remains unvalidated is whether users read these limits correctly. Check stale
facts, unavailable activity records, configuration-only references, command-line
usage without persistent configuration, shared consumers, and out-of-scope projects.
Test whether users can distinguish what was observed from what was inferred.
Universal last-use detection remains deferred rather than promised.

## Lineage and invalidation

Source: 2026-09-19 problem weave, W2a, parent W2, depth 2, G2 (W2a + W2b, all
required); primarily evidence/trust framing. Proposed contributing role is
domain/evidence work; ownership is unassigned. The tactic is now selected in the
[full-scope solution-space session](../sessions/2026-09-19-developer-storage-coverage-and-evidence.md);
implementation and validation remain outstanding.
Reassess if a missing consumer reverses a supposedly supported decision, or users
interpret an old timestamp or missing relationship as permission to delete.
