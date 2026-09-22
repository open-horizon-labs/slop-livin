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

For every module under `crates/core/src/build_adapters/` that is not
`mod.rs`, `matrix.rs`, `registry.rs`, `bounded_io.rs` or the allow-listed
neutral helper `jvm_common.rs`:

1. it references no other adapter module (`super::<other>::`,
   `crate::build_adapters::<other>::`) in any signature or body;
2. no function in `build_adapters/mod.rs`, `actions.rs`, `render.rs`,
   `consumers/cargo.rs`, `crates/cli/src` or `crates/tui/src` contains a
   `match` that names a `*_ADAPTER_ID` constant;
3. `registry.rs` registers each adapter module exactly once (whole path
   segments, so `cargo::Adapter` inside `xcargo::Adapter` is not a hit);
4. every adapter has a row in `matrix.rs`.

**Limits.** The registry check is textual on `registry.rs` because the
registry is a `Vec<Box<dyn BuildAdapter>>` and the resolver cannot follow
trait-object dispatch. Set equality between the registry and the matrix
at *runtime* is the complementary unit test
`registry_ids_are_exactly_the_matrix_ids`, which is what actually proves
the two lists agree.

## Runtime tests that complete it

- `crates/core/src/build_adapters/registry.rs::every_adapter_is_registered_exactly_once`
- `crates/core/src/build_adapters/registry.rs::registry_ids_are_exactly_the_matrix_ids`
- `crates/core/tests/build_adapter_contract.rs`
