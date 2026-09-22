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

Struct-literal *expressions* whose resolved path ends in `NestedArtifact`
or `CandidateNestedUnit` inside an adapter module are rejected, so
`use crate::artifact::NestedArtifact as Unit; Unit { .. }` is caught.
`supported_with_reason("")` / `acts_with_reason("")` -- lifting a default
with an empty reason -- is the same silent override written differently.

**Limits.** A builder call chain that ends in
`.supported_with_reason("because")` is accepted by the audit whatever the
reason says; only review reads the reason. The audit guarantees the
override is visible in the diff.

## Runtime tests that complete it

- `crates/core/tests/build_adapter_contract.rs::builder_defaults_are_inspection_only_and_unsupported`
- `crates/core/tests/build_adapter_contract.rs::aggregate_nodes_carry_no_physical_charge`
