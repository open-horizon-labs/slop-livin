---
id: json-persistence-is-allowlisted
severity: hard
statement: "JSON is a wire and CLI output format, and the format of a fixed set of small control files. It is never a data store. Every code path that serializes JSON into a file must appear in JSON_WRITE_ALLOWLIST with a justification naming a control or recovery artifact."
outcome: disk-growth-by-project
audit: json_persistence_is_allowlisted
---

## Rationale

The sibling guardrail bans JSON *sidecars under the store*; this one
bans the mechanism anywhere, because the sidecars were added one
convenient `serde_json::to_string` at a time. Printing JSON is the
tool's contract with agents and scripts and is untouched; persisting it
is the thing that quietly becomes a database.

## Detection

Two layers, both run by `scripts/check.sh`:

1. AST: in `crates/{core,cli,tui}/src`, a function that calls
   `serde_json::to_vec*`/`to_string*`/`to_writer*`/`Serializer` **and**
   writes (`fs::write`, `write_atomic`, `write_all`, `File::create`,
   `.persist(`, or a function whose own name contains
   write/save/persist) must be in `JSON_WRITE_ALLOWLIST`. Printing to
   stdout or stderr is not persistence and is not flagged. An allow-list
   entry whose file or function no longer matches fails the audit, so
   the list cannot rot.
2. grep: `serde_json::to_(vec|string|writer)` outside the allow-listed
   files, not followed within three lines by a print, fails
   `scripts/check.sh`.

**Limits.** The AST layer's dataflow is "same function body", not a real
taint analysis: serializing in one function and writing in another,
through a struct field, is not caught. The store-contents runtime test
is the backstop.

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

## Runtime tests that complete it

- `crates/core/tests/store_contents_are_allowlisted.rs`
