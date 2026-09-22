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

Detector ids are derived: the constants a `Detector::id()` returns, with their values. Outside the detector implementations' modules, the trait's module and the scope interpreters, no function or item names a detector-id constant, and no function dispatches on two or more detector-id values (match arms or `==` comparisons). The `Detector` trait declares `manager_conventions()`.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `detector_ids_only_in_registry/01-wiring-matches-a-detector-id`, `detector_ids_only_in_registry/02-cli-matches-a-detector-id`, `detector_ids_only_in_registry/03-tui-matches-a-detector-id`, `detector_ids_only_in_registry/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## Runtime tests that complete it

- `crates/core/tests/external_units.rs` and the association tests, which
  assert a new detector's conventions produce associations without a
  wiring edit
