---
id: agent-adapters-are-pluggable
severity: hard
statement: "Agent-tool adapters are independent and statically registered, exactly as location detectors already are. No adapter names another adapter; no central match over tool-id constants decides behaviour; every adapter module is registered exactly once; the registry's tool ids and the support matrix's tool ids are the same set."
outcome: disk-growth-by-project
audit: agent_adapters_are_pluggable
---

## Rationale

Found 2026-09-21: `agents/mod.rs::identify_for_tool` is a fourteen-arm
`match tool_id` over concrete adapter modules; `multi_location_tool`
hardcodes Cline and Roo Code; Aider has a bespoke path threaded through
`discover_and_measure`; and `actions.rs::execute_agent_session_removal`
duplicates the same fourteen arms for re-identification. Adding the
fifteenth tool means editing four unrelated places, and forgetting one
of them produces a tool that identifies but cannot be rechecked at
execution — a safety boundary that silently does not cover a tool.

`agents/pi.rs` additionally fell back to Oh My Pi's header shape. A
change to Oh My Pi's format would then change Pi's identification, for
no reason a reader of either file could see.

Location detectors already solved this with a `Detector` trait and a
static registry. Adapters follow.

## Detection

The tool modules are derived. No tool module (or a child module of one) reaches another by resolved call, value reference, re-export, item-level path or written path; nothing outside the registry and the catalog matches on a tool id (a `match` arm naming one, or two ids named in one function), by constant or by the value it holds; the registry names each tool module's `Adapter` exactly once; the catalog exposes tool ids.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `agent_adapters_are_pluggable/01-adapter-names-another-adapter`, `agent_adapters_are_pluggable/02-central-tool-id-match`, `agent_adapters_are_pluggable/03-duplicate-registration`, `agent_adapters_are_pluggable/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## What replaced the matches

- `agents::AgentAdapter` (`id`, `name`, `capabilities`, `identify`,
  `reidentify`, `project_local_units`) with static registration in
  `agents::registry::Registry::with_builtins`.
- `multi_location_tool`'s hardcoded Cline/Roo Code match became
  `AdapterCapabilities::decomposes_every_location`, declared by the
  adapter.
- Aider's bespoke per-repo path became
  `AdapterCapabilities::project_local_units` plus
  `AgentAdapter::project_local_units`.
- `actions.rs`'s duplicate fourteen-arm match became
  `agents::reidentify_for_tool`, one registry lookup, which runs with
  the identification cache **disabled** so an approval is never spent
  against a cached derivation.
- Pi's fallback to Oh My Pi's header shape is gone: the shared
  byte-offset mechanics live in the neutral `pi_family.rs`, each adapter
  passes only the layouts its own tool documents, and a header an
  adapter cannot parse is an explicit unknown-format outcome.

## Runtime tests that complete it

- `crates/core/tests/agent_matrix_matches_docs.rs` — the published
  support matrix, parsed back and compared with the matrix constant and
  the registry's ids.
- `agents::registry::tests` — exactly-once registration, registry ids ==
  matrix ids, every adapter id is also a detector id, and the two
  capabilities are declared by exactly the adapters that should have
  them (so replacing the hardcoded match did not quietly widen it).
- each adapter's own five required tests (see
  `agent-adapter-test-contract.md`)
