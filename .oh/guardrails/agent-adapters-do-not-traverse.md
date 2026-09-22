---
id: agent-adapters-do-not-traverse
severity: hard
statement: "Adapters do not walk the filesystem. Directory structure reaches them through the folded walk rows in their identification context, or through the capped locations::shallow_list helper for the single-level listings a layout genuinely requires."
outcome: disk-growth-by-project
audit: agent_adapters_do_not_traverse
---

## Rationale

The adapter-scoped half of `no-second-traversal-on-report-path.md`. Each
of the fourteen adapters open-coded its own recursive `read_dir`, so a
report over a large tool home paid for the walk once in the folded walk
and again, per adapter, per call. Keeping this rule adapter-scoped as
well as path-scoped means a new adapter fails the build immediately
rather than quietly re-adding the cost.

## Detection

No adapter function is in the derived traversal set (every function that transitively calls `read_dir`, `walkdir`, `jwalk`), with the closure stopped only at the bounded primitives -- `locations::shallow_list` (named with `SHALLOW_LIST_CAP`) and `folded_measurement::folded_bytes_bounded_stamped` (named with `max_entries`), each of which must name and stop on its cap and do nothing beyond its one bounded operation -- and at the declared-project handoff (functions returning `ProjectLinkState`, which resolve a path read out of a header through the walker's own identity code and may not traverse themselves).

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `agent_adapters_do_not_traverse/01-aliased-read-dir`, `agent_adapters_do_not_traverse/02-plain-read-dir`, `agent_adapters_do_not_traverse/03-walkdir`, `agent_adapters_do_not_traverse/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## Runtime tests that complete it

- `crates/core/tests/incremental_external_and_agent_measurement.rs`
