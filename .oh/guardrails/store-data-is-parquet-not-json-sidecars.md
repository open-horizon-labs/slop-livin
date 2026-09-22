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

For every write anywhere (a primitive, or an exact call to a local writer), the names that make up the written path -- literals in the path argument, in the bindings it derives from, in the local path helpers either calls, and in the constants they name -- must not be a JSON file other than the small control files and patterns.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `store_data_is_parquet_not_json_sidecars/01-format-sidecar`, `store_data_is_parquet_not_json_sidecars/02-plain-sidecar`, `store_data_is_parquet_not_json_sidecars/03-jsonl-sidecar`, `store_data_is_parquet_not_json_sidecars/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## Runtime tests that complete it

- `crates/core/tests/store_contents_are_allowlisted.rs` — a full
  observe → report → propose → approve → execute cycle on a
  multi-ecosystem fixture, then a recursive walk of `SWAMP_DIR`
  asserting every file matches exactly one allow-list pattern, that no
  `.json` exceeds 64 KiB, and that a Trash envelope holds exactly
  `restore.json` plus the moved members.
