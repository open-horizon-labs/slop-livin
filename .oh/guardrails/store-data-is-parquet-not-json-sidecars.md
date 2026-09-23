---
id: store-data-is-parquet-not-json-sidecars
severity: hard
statement: "Per-unit, per-worktree and per-row data -- measurements, identities, associations, identification caches, evidence caches -- lives only in the existing Parquet current + reverse-delta store. JSON under the store directory is limited to a fixed list of small control and recovery files."
outcome: disk-growth-by-project
audit: json_writes_allowlisted
compile_fail:
  - json_writes_name_a_store_file
runtime_tests:
  - crates/core/tests/store_contents_are_allowlisted.rs
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

Mechanism: type, gate audit, runtime test.

**Type.** Every `JsonFile` variant's file name is fixed (or a validated id under a fixed prefix); there is no per-unit or free-named JSON file to write.

**Gate audit.** `json_writes_allowlisted`: no serializer outside the gate's store module.

Retired 2026-09-22: the `store_data_is_parquet_not_json_sidecars` source audit (a `syn` call-graph rule, which four review rounds showed cannot be made mutation-proof without type resolution; `docs/architecture.md`, "Capability gates"). Its mutation fixtures, and the sweep-3 and sweep-4 mutations aimed at it, now run in `crates/source-audit/tests/mutation_sweep.rs`, compiled: each must fail compilation (or clippy) or a gate audit.

Compile-fail cases (`crates/core/tests/compile_fail/`, run by `crates/source-audit/tests/compile_fail.rs` against the production API): `json_writes_name_a_store_file`.

## Runtime tests that complete it

- `crates/core/tests/store_contents_are_allowlisted.rs` — a full
  observe → report → propose → approve → execute cycle on a
  multi-ecosystem fixture, then a recursive walk of `SWAMP_DIR`
  asserting every file matches exactly one allow-list pattern, that no
  `.json` exceeds 64 KiB, and that a Trash envelope holds exactly
  `restore.json` plus the moved members.
