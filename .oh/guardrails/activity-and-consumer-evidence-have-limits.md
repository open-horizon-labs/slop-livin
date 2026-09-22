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

AST audit `activity_and_consumer_evidence_have_limits`, added 2026-09-22
(the 2026-09-22 re-review found this `severity: hard` guardrail with no
`audit:` field at all -- one of the three unwatched ones, and the
unwatched ones were the ones that broke):

- `FactStatus::{Unknown,Unavailable,Conflicting}` may be *constructed*
  only inside `evidence.rs`; everywhere else it comes from
  `Evidence::unknown`/`unavailable`/`conflicting`, whose signatures make
  the reason a required argument. Struct-literal sites are located
  through the AST, so a `match` arm that reads a reason is not mistaken
  for a construction that omits one;
- no call site passes an empty string literal as that reason; and
- every `*_evidence` builder in `activity.rs` names an `EvidenceSource`,
  and `render::render_evidence_lines` mentions `reason`, so an unknown is
  never printed without the limit that makes it readable.

**Limits.** The audit proves the reason exists and is non-empty, not that
it is *informative*. It cannot see a reason assembled at runtime from an
empty variable.

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
