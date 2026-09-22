---
id: agent-adapters-do-not-reach-detectors
severity: hard
statement: "Adapters reference nothing under crate::locations except neutral vocabulary types (StorageCategory, Platform, Provenance). Detector ids and home resolution live in locations/."
outcome: coverage-aware-storage-history
audit: agent_adapters_do_not_reach_detectors
---

## Rationale

Every adapter currently defines its tool id as an alias of its
detector's id constant. That reads harmlessly and creates a cycle:
identification depends on detection's naming, so the registry cannot be
the single place that decides which tool an authorized home belongs to,
and the scope layer's authority over "what is in scope" leaks into the
adapter layer. The review's scope findings all had this shape -- a
decision about scope being re-made somewhere that should only have been
told the answer.

## Detection

Within an adapter, every `locations::<ident>` reference must name one of
`StorageCategory`, `Platform`, `Provenance`.

**Limits.** Re-exports could launder a detector id through another
module; the sibling `detector_ids_only_in_registry` audit covers the
consumer side of the same rule.

## Runtime tests that complete it

- `crates/core/tests/agent_matrix_matches_docs.rs`
