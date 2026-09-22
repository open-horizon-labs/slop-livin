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

No adapter function's signature or body, and no adapter item-level declaration (const, static, type alias, field), names a path into `locations::` other than the shared vocabulary -- derived as the `locations` data types that the shared agent model's own fields carry -- and the bounded lister. A detector module, a detector id, the `Detector` trait, `Environment` and `Registry` are detector identity.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `agent_adapters_do_not_reach_detectors/01-adapter-reads-detector-ids`, `agent_adapters_do_not_reach_detectors/02-adapter-names-a-detector-type`, `agent_adapters_do_not_reach_detectors/03-aliased-locations-reach`, `agent_adapters_do_not_reach_detectors/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## Runtime tests that complete it

- `crates/core/tests/agent_matrix_matches_docs.rs`
