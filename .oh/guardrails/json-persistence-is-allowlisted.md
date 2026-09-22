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

A function persists JSON when a serialized value (`serde_json::to_*` other than `to_value`, or `json!(..).to_string()`, however the serializer is imported) reaches a write: as a write's argument, through a binding (including `if let Ok(x) = ..`), `to_writer` into a file, or `write!`/`writeln!` into anything but a terminal or a string buffer. Every such function must be on the allow-list, and every allow-list entry must still do it.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `json_persistence_is_allowlisted/01-json-macro-to-string`, `json_persistence_is_allowlisted/02-serializer-outside-allowlist`, `json_persistence_is_allowlisted/03-aliased-writer`, `json_persistence_is_allowlisted/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

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
