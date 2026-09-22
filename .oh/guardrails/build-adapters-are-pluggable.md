---
id: build-adapters-are-pluggable
severity: hard
statement: "A build-artifact adapter is one line in a static registry, names no other adapter, and nothing dispatches to it by matching an ecosystem id. Registry ids, capability-matrix ids and the published docs table are the same set."
outcome: disk-growth-by-project
audit: build_adapters_are_pluggable
---

## Rationale

The agent side reached fourteen adapters before anyone noticed that
`agents/mod.rs::identify_for_tool` was a fourteen-arm `match tool_id`,
that `actions.rs` held a second copy of it, and that adding a tool meant
editing four places -- so a tool that identified fine could still refuse
to re-verify at execution because one of the four was forgotten.

The build side was one adapter away from the same shape on 2026-09-22:
`consumers/cargo.rs` called `cargo_artifacts::folded_units` by name, and
the obvious way to add Node was a second named call, then a
`match ecosystem` once there were four. This guardrail landed *before*
the second adapter, not after the fourth.

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

1. No governed file except the derived registry (the one defining
   `with_builtins`) references an adapter module -- resolved paths, so
   `use super::{gradle}`, `use crate::build_adapters::cargo as x`,
   `super::gradle::` and paths inside macros all count. A neutral helper
   naming an adapter is a violation too: it would be a back door between
   two adapters.
2. Across **every** file in `crates/{core,cli,tui}/src` (registry and
   matrix excepted): no `match` on a `*_ADAPTER_ID` constant; no `match`
   whose scrutinee mentions `adapter` with an arm matching an adapter id
   literal; no `==`/`!=` whose operands, or the method-chain receiver it
   sits in (`u.adapter.as_deref().filter(|a| *a != "cargo")`), mention
   `adapter` and whose text holds an adapter id literal or a `const`
   holding one; no `if let` / `matches!` of the same shape.
3. `with_builtins` registers each derived `<module>::<Type>` exactly once.
4. The adapters' ids equal the matrix's `Status::Implemented` ids, both
   directions.

**Limits.** The call graph is lexical: trait-object dispatch
(`adapter.identify(..)` through `dyn BuildAdapter`), function pointers
stored in a struct and closures passed across modules are not followed,
and a method call resolves only to the `impl`s of types the calling
function names in its signature or body. A primitive *named* anywhere in
a governed function (a function pointer, a type) is flagged even when it
is not called. Unknown macros in a governed module fail the rule. Dispatch through the registry by
a literal (`registry.get("node")`) is not flagged: it goes through the
registry.

## Runtime tests that complete it

- `crates/core/src/build_adapters/registry.rs::every_adapter_is_registered_exactly_once`
- `crates/core/src/build_adapters/registry.rs::registry_ids_are_exactly_the_matrix_ids`
- `crates/core/tests/build_adapter_contract.rs`
