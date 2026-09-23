---
id: json-persistence-is-allowlisted
severity: hard
statement: "JSON is a wire and CLI output format, and the format of a fixed set of small control files. It is never a data store. Every code path that serializes JSON into a file must appear in JSON_WRITE_ALLOWLIST with a justification naming a control or recovery artifact."
outcome: disk-growth-by-project
audit: json_writes_allowlisted, gate_paths_only_inside_gates
compile_fail:
  - json_writes_name_a_store_file
  - atomic_write_is_private
  - store_files_live_in_a_store_dir
  - logs_are_not_caller_paths
runtime_tests:
  - crates/core/tests/store_contents_are_allowlisted.rs
  - crates/core/tests/token_binding.rs::the_ledger_appends_to_nothing_but_a_ledger
  - crates/core/tests/token_binding.rs::a_store_dir_is_a_real_absolute_directory
---

## Rationale

The sibling guardrail bans JSON *sidecars under the store*; this one
bans the mechanism anywhere, because the sidecars were added one
convenient `serde_json::to_string` at a time. Printing JSON is the
tool's contract with agents and scripts and is untouched; persisting it
is the thing that quietly becomes a database.

## Detection

Mechanism: type, gate audit, runtime test.

**Type.** JSON reaches disk only through `fs_gate::store::write_json(JsonFile, ..)`; `JsonFile` is the allow-list (a new control file is a new variant), and the atomic byte writer is private. **Locations are types too** (re-review 5, finding 4): every variant is a fixed name inside a typed `StoreDir` (the resolved swamp dir, or -- in the store modules only -- `StoreDir::at`, which refuses a relative path, a symlink and a non-directory); the ledger appends only to a store's `ledger.jsonl` (or the `SWAMP_LEDGER_PATH` override, read in the gate); the observation log must be named `observe.log`; the LaunchAgent plist is resolved in the gate; and there is no free `create_dir_all(path)` or `list_owned(path)`. The one non-JSON store file this chunk added is `authority.key` (32 random bytes, `fs_gate::key`).

**Gate audit.** `json_writes_allowlisted`: no serde serializer that produces bytes or text (`to_vec*`, `to_string*`, `to_writer*`, `Value`'s `Display`) outside the gate's store module; `std::fs` writes are gate paths.

Retired 2026-09-22: the `json_persistence_is_allowlisted` source audit (a `syn` call-graph rule, which four review rounds showed cannot be made mutation-proof without type resolution; `docs/architecture.md`, "Capability gates"). Its mutation fixtures, and the sweep-3 and sweep-4 mutations aimed at it, now run in `crates/source-audit/tests/mutation_sweep.rs`, compiled: each must fail compilation (or clippy) or a gate audit.

Compile-fail cases (`crates/core/tests/compile_fail/`, run by `crates/source-audit/tests/compile_fail.rs` against the production API): `json_writes_name_a_store_file`, `atomic_write_is_private`, `store_files_live_in_a_store_dir`, `logs_are_not_caller_paths`.

## Allow-list and why each entry is a control artifact

| File | Function | Why |
| --- | --- | --- |
| `actions.rs` | `save_plan` | one unapproved plan per file; a control artifact a human reviews |
| `actions.rs` | `write_restore_manifest` | Trash envelope recovery manifest, written beside the moved members |
| `grants.rs` | `write_grants` | the authorization grant list: small, human-auditable |
| `ledger.rs` | `append` | append-only action ledger (jsonl): a record, not a queryable table |
| `schedule.rs` | `write_last_run` | a single timestamp marker |
| `scope.rs` | `persist_effective_scope` | resolved-scope snapshot for the next run's coverage diff |
| `agents/mod.rs` | `save_protect` | human keep/protect list, written atomically |
| `cargo_cleanup.rs` | `move_reviewed` | Trash envelope recovery manifest for one reviewed Cargo group |
| `continuity.rs` | `write_checkpoint` | a Linux collector's checkpoint (#82): epoch, coverage and a dirty list bounded by `live_watch::DIRTY_BOUND` -- one small control file per watched root, never an inventory |

## Runtime tests that complete it

- `crates/core/tests/store_contents_are_allowlisted.rs`
