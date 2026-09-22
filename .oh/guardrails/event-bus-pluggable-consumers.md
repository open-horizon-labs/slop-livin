---
id: event-bus-pluggable-consumers
severity: hard
statement: "The report pipeline is consumers on an in-memory event bus (tokio runtime, static registration, dynamic routing): no consumer imports another, no consumer registers consumers at runtime, and every report is assembled by EventBus::run."
outcome: disk-growth-by-project
audit: event_bus_pluggable_consumers
---

## Rationale
This is the repo-native-alignment architecture the tool was specified to follow, and it was not built the first time: `report_full_mode_with_source` grew into a thousand-line hardwired stage chain and each new enrichment (git signals, GitHub, Docker, ecosystems, tracking) was bolted into it. The bus is the only coupling between stages, so a new fact source is one file that registers itself.

## Detection

The composition of `no_consumer_knows_other_consumers`, `static_registration_only`, `extractors_are_pluggable` and `all_report_paths_through_bus`, over the derived consumer set (every implementor of `bus::Consumer`, wherever written, including `consumers/mod.rs`).

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `event_bus_pluggable_consumers/01-consumer-names-the-bus`, `event_bus_pluggable_consumers/02-runtime-registration`, `event_bus_pluggable_consumers/03-report-calls-a-stage-directly`, `event_bus_pluggable_consumers/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

