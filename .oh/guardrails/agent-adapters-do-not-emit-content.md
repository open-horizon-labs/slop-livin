---
id: agent-adapters-do-not-emit-content
severity: hard
statement: "Adapters return data; they never print, log or trace. Rendering happens in the shared layer, which is the only place that knows what may be shown."
outcome: decision-relevant-storage-evidence
audit: agent_adapters_do_not_emit_content
---

## Rationale

The privacy contract is about what leaves the process, not only about
what enters a struct field. An adapter debugging a header parse with
`eprintln!("{line}")` would print a conversation's first line to the
terminal and into any CI log — past every redaction the rendering layer
applies, and past the canary assertions, which check returned values
rather than stdout.

## Detection

No adapter function is in the derived emitter set: every function that transitively prints (`print*!`, `eprint*!`, `dbg!`), logs (`log::*`, `tracing::*`), writes to `io::stdout`/`io::stderr`, or panics with a formatted message (`panic!`/`unreachable!`/`assert*!` with a placeholder or a `display()`, `expect` with a formatted argument).

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `agent_adapters_do_not_emit_content/01-writeln-stderr`, `agent_adapters_do_not_emit_content/02-plain-eprintln`, `agent_adapters_do_not_emit_content/03-print-to-stdout`, `agent_adapters_do_not_emit_content/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## Runtime tests that complete it

- every adapter's `canary_content_never_appears_in_output`
