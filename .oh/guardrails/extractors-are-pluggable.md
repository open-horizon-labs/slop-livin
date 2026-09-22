---
id: extractors-are-pluggable
severity: hard
statement: "A new fact source (a Docker extractor, a GitHub enricher, an ecosystem detector) is a consumer registered in EventBus::with_builtins; it never requires a change to the walk or to report assembly."
outcome: disk-growth-by-project
audit: extractors_are_pluggable
---

## Detection

Every derived consumer is boxed for registration exactly once in the production program, in the bus's static registrar; outside the consumer modules and the registrar's module, nothing calls, references, or names a path into a consumer module.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `extractors_are_pluggable/01-unregistered-consumer`, `extractors_are_pluggable/02-walker-reaches-into-consumers`, `extractors_are_pluggable/03-aliased-unregistered-consumer`, `extractors_are_pluggable/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

