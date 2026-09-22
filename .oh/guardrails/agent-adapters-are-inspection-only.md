---
id: agent-adapters-are-inspection-only
severity: hard
statement: "Identification never acts. An adapter declares an action capability on the unit it returns; only the shared sink -- with the plan, grant, recheck, ledger and Trash discipline -- executes anything. Adapters reference no action, plan, grant, ledger or filesystem-mutating API."
outcome: decision-relevant-storage-evidence
audit: agent_adapters_are_inspection_only
---

## Rationale

"Inspection is not authorization" is a handoff constraint, and the whole
safety argument for agent storage rests on a single sink where identity,
protection and occupancy are rechecked. Fourteen adapters, each free to
call `fs::rename`, would be fourteen places that argument has to be
re-made. The review's counterexamples were all failures *at* the sink;
they would have been unrecoverable if the sink were not the only way
through.

## Detection

No adapter function is in the derived mutating set: every function that transitively calls a `std::fs` primitive that replaces, removes, copies, links or re-permissions bytes, `File::create`, `OpenOptions` opened for `write`/`truncate`/`append`, `trash::*`, a persisted temp file, `create_dir*`, or a subprocess.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `agent_adapters_are_inspection_only/01-remove-dir-all-in-identify`, `agent_adapters_are_inspection_only/02-aliased-remove`, `agent_adapters_are_inspection_only/03-plan-in-adapter`, `agent_adapters_are_inspection_only/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## Runtime tests that complete it

- `crates/core/tests/execution_rechecks.rs` — every destructive path in
  the test suite goes through the sink
