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

1. No adapter module references another by path (`super::<adapter>::`,
   `crate::agents::<adapter>::`). `vscode_family.rs`, `pi_family.rs` and
   `bounded_io.rs` are neutral helpers that name no tool and are exempt.
2. Outside the registry, no function in `agents/mod.rs`, `actions.rs` or
   `crates/tui/src/**` may `match` on a `*_TOOL_ID` constant.
3. Every adapter module is registered in `agents/registry.rs` exactly
   once (module set ↔ registration set equality).
4. The registry's ids and `agents::matrix`'s ids are the same set.

**Limits.** Check 2 is "contains a match and the constant", so a
tool-id match spelled without the constant would slip past; check 4
compares id *sets*, not capabilities, which the matrix↔docs test covers.

## Runtime tests that complete it

- `crates/core/tests/agent_matrix_matches_docs.rs`
- each adapter's own five required tests (see
  `agent-adapter-test-contract.md`)
