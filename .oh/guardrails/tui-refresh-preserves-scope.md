---
id: tui-refresh-preserves-scope
severity: hard
statement: "Every TUI observation -- startup, background refresh, live watch, post-action re-observe -- goes through the scope-aware report path. Excluded subtrees and pruned external locations stay absent on refresh, and external/agent unit vectors are refreshed from the same observation."
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

**Limits.** The audit is about which entry point is called; it cannot
tell whether the scope forwarded is the right one.

## Runtime tests that complete it

- `crates/tui/tests/scope_preserving_refresh.rs` — an excluded subtree
  and an external location pruned from its parent root stay absent after
  a background refresh and after a successful action; `prune_removed`
  removes exactly the successful agent/external rows.
