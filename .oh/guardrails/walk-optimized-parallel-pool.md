---
id: walk-optimized-parallel-pool
severity: soft
statement: "The full walk runs on the work-stealing pool with folded units sized as parallel Size jobs; no serial walk on the report path."
outcome: disk-growth-by-project
audit: walk_optimized_parallel_pool
---

## Rationale
15 s → 7.5 s on ~/src came from the pool and from sizing folded units in parallel. The serial `attribution::attribute` walk exists for tests only.

## Detection

`walk::discover_and_attribute` reaches the function that drains the worker pool, and the serial `attribution::attribute` has no production caller.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `walk_optimized_parallel_pool/01-serial-walk-on-report-path`, `walk_optimized_parallel_pool/02-serial-walk-in-a-consumer`, `walk_optimized_parallel_pool/03-aliased-serial-walk`, `walk_optimized_parallel_pool/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

