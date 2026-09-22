---
id: agent-adapters-are-environment-free
severity: hard
statement: "An adapter knows only the home it is handed. It never reads the process environment, never resolves a home directory, and hardcodes no user path."
outcome: coverage-aware-storage-history
audit: agent_adapters_are_environment_free
---

## Rationale

Home resolution -- including every documented override variable -- is a
detector's job, and detectors are already fixture-injectable through
`locations::Environment`. An adapter that reads `HOME` itself is
untestable without touching the real machine, which is exactly what the
privacy rule forbids: fixtures must be synthetic, and no test may read a
real tool home.

## Detection

No adapter function is in the derived environment-reading set (any function that transitively calls `std::env::*`, `dirs::*`, `home::*` or a `home_dir`), and no adapter function or item-level declaration holds a hardcoded home (`"HOME"`, `/Users/`, `/home/`). Adapters include the shared family modules (`vscode_family`, `pi_family`, `bounded_io`).

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `agent_adapters_are_environment_free/01-aliased-env-var`, `agent_adapters_are_environment_free/02-plain-env-var`, `agent_adapters_are_environment_free/03-hardcoded-home-literal`, `agent_adapters_are_environment_free/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## Runtime tests that complete it

- every adapter's own tests, all of which inject a `tempfile` home
