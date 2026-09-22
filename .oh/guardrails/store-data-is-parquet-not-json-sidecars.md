---
id: store-data-is-parquet-not-json-sidecars
severity: hard
statement: "Per-unit, per-worktree and per-row data -- measurements, identities, associations, identification caches, evidence caches -- lives only in the existing Parquet current + reverse-delta store. JSON under the store directory is limited to a fixed list of small control and recovery files."
outcome: disk-growth-by-project
audit: store_data_is_parquet_not_json_sidecars
---

## Rationale

The handoff's hard constraint is explicit: "No new SQLite store, giant
JSON artifact cache, parallel database or per-file persistent
inventory." The stack nevertheless introduced `external_consumers.json`
(external.rs) and `toolchain_declarations_cache.json` /
`dependency_identities_cache.json` (consumer wiring and external
associations). Each is per-unit data, grows without bound, is rewritten
whole on every change, and cannot be compacted or retained by the
existing reverse-delta machinery — a parallel database with a JSON
syntax.

## Detection

Every string literal ending in `.json`, `.jsonl` or `.json.zst` in
`crates/core/src` and `crates/tui/src` must name one of
`STORE_CONTROL_FILES`: grants, the ledger, the last-run marker, the
FSEvents cursor, topology, the Docker facts cache, the unowned cache, UI
state, the scope snapshot, the protect list, a Trash envelope's
`restore.json`, and the compressed last-report cache. Paths built by
`format!` (plan files, `plans/<id>.json`) are exempt.

**Limits.** A literal-based check; a filename assembled from fragments
would slip past it. The runtime test walks the actual store directory,
which is the real guarantee.

## Runtime tests that complete it

- `crates/core/tests/store_contents_are_allowlisted.rs` — a full
  observe → report → propose → approve → execute cycle on a
  multi-ecosystem fixture, then a recursive walk of `SWAMP_DIR`
  asserting every file matches exactly one allow-list pattern, that no
  `.json` exceeds 64 KiB, and that a Trash envelope holds exactly
  `restore.json` plus the moved members.
