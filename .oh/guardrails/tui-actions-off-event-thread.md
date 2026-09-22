---
id: tui-actions-off-event-thread
severity: hard
statement: "Cleanup review and execution must not run synchronously on the TUI event/render path; workers report progress and stop between groups."
outcome: disk-growth-by-project
audit: tui_actions_off_event_thread
---

## Rationale

A 617-group cleanup froze the UI for 82 seconds. Correct filesystem outcomes
did not make a frozen confirmation screen acceptable. Group marking can perform
expensive checks too; both review and execution belong on background workers.

## Detection

From every TUI entry point (the required ones, and every function named for key handling, the event loop or drawing), follow exactly resolved calls and value references, never into a `thread::spawn` closure. A TUI function blocks if it builds a subprocess, sleeps, waits on a channel or child, or joins a worker; a core callee blocks if it is in the closure of those plus traversal (stopped at the capped `shallow_list`). Calls inside macro arguments are calls.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `tui_actions_off_event_thread/01-probe-path-per-keystroke`, `tui_actions_off_event_thread/02-command-output-on-event-path`, `tui_actions_off_event_thread/03-aliased-blocking-report`, `tui_actions_off_event_thread/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## Limits and runtime checks

This is a conservative name-based graph, not compiler type resolution. It can
conflate same-named methods; macros, dynamic dispatch, new external blocking APIs,
local import aliases and indirect worker handles need review. Join checks recognize
receivers named with `handle` or `worker`, not all possible JoinHandle values;
string joining remains allowed. The explicit sink list and worker-boundary syntax
must evolve with the architecture. This audit does not claim all UI I/O is absent.

Keep runtime tests for cancellation, partial outcomes, ledger preservation and
responsive progress rendering. Static structure alone cannot prove responsiveness
or correct cancellation semantics.
