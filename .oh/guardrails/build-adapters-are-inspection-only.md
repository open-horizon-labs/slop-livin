---
id: build-adapters-are-inspection-only
severity: hard
statement: "A build adapter identifies from read-only metadata. It never renames, writes or deletes, never reaches the plan/grant/ledger layer, and never spawns a process -- `npm`, `gradle`, `mvn` and `cargo` all evaluate the project's own build definition to answer a question."
outcome: disk-growth-by-project
audit: build_adapters_are_inspection_only
---

## Rationale

Two hard constraints meet here. "Inspection is not authorization": the
adapters in this epic ship identification with no cleanup at all, so an
adapter that can delete is a capability nobody reviewed. And "do not run
untrusted project hooks/config/builds during observation": the four
commands that would answer an adapter's questions most directly --
`npm ls --json`, `gradle dependencies`, `mvn help:evaluate`,
`cargo metadata` -- each execute the project's own build definition, which
is arbitrary code from whatever the user happened to clone.

The 2026-09-22 mutation sweep put `fs::remove_dir_all(home.join("logs"))`
into an agent adapter's `identify` and the then-current
"inspection only" audit passed it, because `remove_dir_all` was simply
missing from a hand-written token list. This audit resolves calls instead
and lists every mutating primitive.

## Detection

Resolved production calls in adapter modules may not reach
`fs::{rename,remove_file,remove_dir,remove_dir_all,write,create_dir,
create_dir_all,set_permissions,copy,hard_link}`,
`Command::{new,output,status,spawn}` or `process::Command`; the token
scan additionally rejects `actions::`, `trash::`, `Plan {`, `Grant {`
and `Ledger`. Aliases are resolved, so `use std::process::Command as
Runner` is reported as `Command::new`. A discarded result
(`let _ = remove_dir_all(..)`) is the same call.

**Limits.** A call reached through a trait object or a function pointer
is invisible to the resolver. The runtime complement is the per-adapter
`no_project_or_build_code_is_executed` test, which identifies a fixture
containing an executable named after the ecosystem's build tool and
asserts the process-spawn counter stayed at zero.

## Runtime tests that complete it

- `build_adapters/{cargo,node,gradle,maven}.rs::no_project_or_build_code_is_executed`
- `crates/core/tests/build_adapter_contract.rs::identification_spawns_no_processes`
