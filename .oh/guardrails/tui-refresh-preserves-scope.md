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

No function under `crates/tui/src` may call a report entry point that
takes a bare root and no `EffectiveScope`/pruned-subtree argument
(`report_with`, `report_with_dirs`, `report_with_observe`,
`report_with_enrich`, `report_full*`, `report_single_root`). Only the
scope-carrying entries, or a TUI wrapper that forwards the scope, are
allowed.

Since 2026-09-22 the audit also checks *what gets replaced*. The
independent re-review's CE3: `observe_live` correctly narrows the scope
to the one root a watcher fired under (`EffectiveScope::restricted_to`)
and then asked `observe_scope` for `ObservationParts::ALL`, so
`agent_units`/`external_units` came back derived from that one-root
scope and the event loop applied them unconditionally. The agent view
emptied on the first file save anywhere in the project. A *correctly*
narrowed scope must not be allowed to narrow what is replaced, so a
narrowed refresh must request `ObservationParts::WALK_ONLY` and return
`None` for both unit vectors. The audit requires exactly that of
`App::observe_live`.

**Limits.** The audit is about which entry point is called and which
parts a narrowed refresh asks for; it cannot tell whether the scope
forwarded is the right one.

## Runtime tests that complete it

- `crates/tui/tests/scope_preserving_refresh.rs` — an excluded subtree
  and an external location pruned from its parent root stay absent after
  a background refresh and after a successful action; `prune_removed`
  removes exactly the successful agent/external rows.
- `crates/tui/tests/reviewer_counterexamples_stack2_tui.rs::a_live_refresh_of_one_root_must_not_empty_the_agent_view`
  — the 2026-09-22 CE3, driving the real `observe_live` and applying its
  result exactly as `tui::lib`'s event loop does.
