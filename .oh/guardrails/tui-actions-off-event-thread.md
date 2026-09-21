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

The `syn` AST audit starts at `event_loop`, key dispatchers and `draw`, follows
local free functions and methods across TUI source files, and rejects synchronous
marking, proposal, execution, free-space probes, sleep and blocking channel waits.
Ordinary closures remain in scope. Only a literal `std::thread::spawn` closure
is excluded; expressions constructing its argument are still checked. Renamed
imports and function-path references are followed. Missing entry points fail.

Run `cargo run -p swamp-source-audit -- tui_actions_off_event_thread`.
The same repository check runs under `cargo test -p swamp-source-audit`, so the
existing workspace test/release workflow executes it. Negative fixtures cover
direct calls, helper indirection, ordinary closures, renamed imports, function
references, blocking receives, eager spawn arguments and immediate handle joins.

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
