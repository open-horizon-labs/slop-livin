---
id: reverse-delta-current-plus-deltas
severity: hard
statement: "The store holds one current-state file plus a reverse-delta log; an observation rewrites current and appends the previous values of changed rows as a delta, for artifacts, directories and files alike."
outcome: disk-growth-by-project
audit: reverse_delta_current_plus_deltas
---

## Rationale
Reverse deltas make the latest state a single read and history a replay backwards, which is what a sparkline or a growth window needs. The DuckDB→Go port once discarded this design; it does not get discarded again.

## Detection

A history writer is derived: a function that computes a current-table path and is in the destructive set. It must honour a call to a delta-path helper. Every `observe_and_annotate*` reaches both a current-table and a delta-path helper.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `reverse_delta_current_plus_deltas/01-no-delta-append`, `reverse_delta_current_plus_deltas/02-delta-only-no-current`, `reverse_delta_current_plus_deltas/03-files-history-loses-both`, `reverse_delta_current_plus_deltas/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

