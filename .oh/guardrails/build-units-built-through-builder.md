---
id: build-units-built-through-builder
severity: hard
statement: "Adapters build nested units with `NestedUnitBuilder`, never a `NestedArtifact { .. }` literal. The constructor applies the role vocabulary, the accounting basis, the timestamp provenance and `InspectionOnly`."
outcome: disk-growth-by-project
audit: build_units_built_through_builder
---

## Rationale

Every field a literal has to spell out has a safe default that a literal
can silently get wrong, and each wrong value fails in the direction that
*overstates* what swamp knows:

- `coverage.supported: true` on a layout nobody tested -- a support claim
  with no fixture behind it;
- `Membership::Exclusive` on a hardlinked or content-addressed store
  entry -- double counting, which is exactly what #65 forbids;
- a nonzero `physical_total` on an aggregate node -- double counting
  again, one level up;
- an action capability other than `InspectionOnly` -- a promise no build
  adapter can keep, because none of them implements an action.

The agent-side precedent is literal: `AgentUnitBuilder` exists because a
`CandidateAgentUnit { protected: false, .. }` literal could ship a
credentials file unprotected and nothing noticed.

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

1. No `NestedArtifact`, `ArtifactCoverage`,
   `NestedActionCapability::Unsupported` or `CandidateNestedUnit` struct
   literal in a governed module outside `impl NestedUnitBuilder`
   (resolved, so `use NestedArtifact as Unit` counts).
2. No write to a unit field after `build()` in a governed module outside
   `impl NestedUnitBuilder`. The field set is parsed from
   `artifact.rs` (`NestedArtifact` and its `ArtifactCoverage`), so a new
   field is governed the day it is added. A write is an assignment, a
   compound assignment (`+=`), a `&mut` borrow (`mem::replace`,
   `mem::take`), a `ref mut` binding, or any method on the field not in
   a fail-closed read-only allow-list (`push`, `clear`, `retain`,
   `iter_mut` are writes) -- inside macro arguments too. `self.<field>`
   inside another type's `impl` is that type's field, not a unit's.
3. A function anywhere else in the workspace that takes a unit mutably
   and writes a field, or builds a `NestedArtifact` literal, taints every
   governed function that reaches it (`cargo_artifacts::inspect_target`
   builds literals, so an adapter calling it fails).
4. No `supported_with_reason`/`no_action_because` with an empty reason.

Enrichment re-opens a unit: `NestedUnitBuilder::amend(unit)` then named
methods (`role`, `limit`, `modified_at_least`, `complete_only_if`, ...),
which the corpus accepts (`09-amend-is-accepted`).

**Limits.** The call graph is lexical: trait-object dispatch
(`adapter.identify(..)` through `dyn BuildAdapter`), function pointers
stored in a struct and closures passed across modules are not followed,
and a method call resolves only to the `impl`s of types the calling
function names in its signature or body. A primitive *named* anywhere in
a governed function (a function pointer, a type) is flagged even when it
is not called. Unknown macros in a governed module fail the rule. The field match is by name, not by type:
a governed function writing a field that happens to share a unit
field's name on another type is flagged (a false positive the author
fixes by a rename or a builder method, never a miss).

## Runtime tests that complete it

- `crates/core/tests/build_adapter_contract.rs::builder_defaults_are_inspection_only_and_unsupported`
- `crates/core/tests/build_adapter_contract.rs::aggregate_nodes_carry_no_physical_charge`
