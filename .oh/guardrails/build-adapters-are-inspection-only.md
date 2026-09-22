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

All eight build-adapter rules range over **derived** sets
(`crates/source-audit/src/build_audits.rs`, module doc): the governed
modules are every file under `crates/core/src/build_adapters/` --
`mod.rs`, `registry.rs`, `matrix.rs`, `jvm_common.rs` and `bounded_io.rs`
included, no file exempt by name -- plus any workspace file holding an
`impl BuildAdapter for ..`; adapters, their types and their ids are read
from those impls. Re-review 3 (`review/REVIEW-STACK-3.md` section 1)
found 31 of 43 audit slips were a hand-written list that did not contain
the thing; these rules keep no such list except the four bounded
primitives, each of which is itself checked to name its cap.

Every function in a governed module fails if it **reaches**, through the
whole-workspace call graph, a filesystem write (`fs::{rename,
remove_file, remove_dir, remove_dir_all, write, create_dir[_all],
set_permissions, copy, hard_link, soft_link, symlink}`, `File::create`),
or any member of the namespaces `OpenOptions`, `Command`, `process`,
`trash`, `actions`, `grants`, `ledger`, `execution`, `cargo_cleanup` --
directly, one call away in a helper in another module
(`crate::growth::touch_folded_rows`), through a `BuildCtx` method in
`mod.rs`, inside a macro's arguments, or named as a value.

**Limits.** The call graph is lexical: trait-object dispatch
(`adapter.identify(..)` through `dyn BuildAdapter`), function pointers
stored in a struct and closures passed across modules are not followed,
and a method call resolves only to the `impl`s of types the calling
function names in its signature or body. A primitive *named* anywhere in
a governed function (a function pointer, a type) is flagged even when it
is not called. Unknown macros in a governed module fail the rule.

## Runtime tests that complete it

- `build_adapters/{cargo,node,gradle,maven}.rs::no_project_or_build_code_is_executed`
- `crates/core/tests/build_adapter_contract.rs::identification_spawns_no_processes`
