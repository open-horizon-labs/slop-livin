---
id: human-only-authorization
severity: hard
statement: "Authorization (approve a plan, add or revoke a grant) is minted from exactly two reviewed call sites -- the CLI's approve/grant command handling and the TUI's confirmed-execution path -- and nowhere else in the source. This is a code-review boundary, not a human-identity boundary: any shell-capable process, including an agent, can invoke the CLI subcommands that reach those call sites. The sink (grant scope/budget/expiry checks, per-unit re-derivation at execute time, the ledger) is what actually limits what an authorized action can do, independent of who or what invoked it."
outcome: disk-growth-by-project
audit: gate_paths_only_inside_gates
compile_fail:
  - authorized_is_minted_only_by_authorize
  - human_confirmation_is_not_a_struct_literal
  - approval_without_a_confirmation_does_not_exist
runtime_tests:
  - crates/core/tests/execution_rechecks.rs
  - crates/core/tests/actions_r7.rs
---

## Why this changed

Through v0.6.x this guardrail read "the MCP server has no code path that
approves a plan or writes a grant" and the audit checked exactly one
file (`crates/mcp/src/main.rs`) for the absence of those calls. That was
true, and it was also easy to mis-read as "an agent cannot mint
authorization" -- never true, and less relevant once MCP was removed
(#104) and the CLI became the only interface: nothing distinguishes a
human typing `swamp approve <plan_id>` from a script or an LLM agent
running the identical command. See
`skills/swamp/references/trust-model.md` for the full explanation this
guardrail is deliberately kept consistent with.

## Detection

Mechanism: type, gate audit, runtime test.

**Type.** Approving a plan (`actions::approve_confirmed`) and minting a standing grant (`add_standing_grant_confirmed`) take a `&HumanConfirmed`; destructive calls take an `Authorized` from `authority::authorize` (a live grant) or `authorize_confirmed` (a TUI confirmation). Neither token has a public constructor besides these.

**Gate audit.** `gate_paths_only_inside_gates` pins the mint sites: `HumanConfirmed::cli_command` only in the CLI, `HumanConfirmed::tui_dialog` only in the TUI's `app` (its confirm dialog), `authorize` only in `actions`, `authorize_confirmed` only in the TUI sink.

Retired 2026-09-22: the `human_only_authorization` source audit (a `syn` call-graph rule, which four review rounds showed cannot be made mutation-proof without type resolution; `docs/architecture.md`, "Capability gates"). Its mutation fixtures, and the sweep-3 and sweep-4 mutations aimed at it, now run in `crates/source-audit/tests/mutation_sweep.rs`, compiled: each must fail compilation (or clippy) or a gate audit.

Compile-fail cases (`crates/core/tests/compile_fail/`, run by `crates/source-audit/tests/compile_fail.rs` against the production API): `authorized_is_minted_only_by_authorize`, `human_confirmation_is_not_a_struct_literal`, `approval_without_a_confirmation_does_not_exist`.
