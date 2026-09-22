---
id: tui-refresh-preserves-scope
severity: hard
statement: "Every TUI observation -- startup, background refresh, live watch, post-action re-observe -- goes through the scope-aware report path. Excluded subtrees and pruned external locations stay absent on refresh. External/agent unit vectors are refreshed only from an observation that covered the whole scope; a refresh narrowed to one root asks for no unit parts and leaves those vectors alone."
outcome: coverage-aware-storage-history
audit: tui_refresh_preserves_scope
---

## Rationale

Two P1 findings in the 2026-09-21 review:

- Startup used `report_scope_with_parts` with exclusions and external
  pruning, but `app.rs`'s background and live refreshes called
  single-root report functions that default `pruned_subtrees` to empty.
  A refresh could therefore reintroduce excluded data and double-count
  external bytes that startup had pruned.
- `finish_startup` was the only production caller updating the
  external/agent unit vectors, and `prune_removed` did not remove their
  successful action rows, so agent storage could stay stale while the
  header claimed the report was live.

The scope contract has to survive every update, not just the first
render.

## Detection

No TUI function calls a public function of the report module (derived: the modules that run the bus, with child modules) that takes a bare root and no scope, unless it only loads a stored report back; a call into the report module that resolves to nothing fails.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `tui_refresh_preserves_scope/01-scopeless-report-with`, `tui_refresh_preserves_scope/02-scopeless-full-mode`, `tui_refresh_preserves_scope/03-scopeless-single-root`, `tui_refresh_preserves_scope/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## Runtime tests that complete it

- `crates/tui/tests/scope_preserving_refresh.rs` — an excluded subtree
  and an external location pruned from its parent root stay absent after
  a background refresh and after a successful action; `prune_removed`
  removes exactly the successful agent/external rows.
- `crates/tui/tests/reviewer_counterexamples_stack2_tui.rs::a_live_refresh_of_one_root_must_not_empty_the_agent_view`
  — the 2026-09-22 CE3, driving the real `observe_live` and applying its
  result exactly as `tui::lib`'s event loop does.
