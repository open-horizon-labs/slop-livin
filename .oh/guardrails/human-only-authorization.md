---
id: human-only-authorization
severity: hard
statement: "Authorization (approve a plan, add or revoke a grant) is minted from exactly two reviewed call sites -- the CLI's approve/grant command handling and the TUI's confirmed-execution path -- and nowhere else in the source. This is a code-review boundary, not a human-identity boundary: any shell-capable process, including an agent, can invoke the CLI subcommands that reach those call sites. The sink (grant scope/budget/expiry checks, per-unit re-derivation at execute time, the ledger) is what actually limits what an authorized action can do, independent of who or what invoked it."
outcome: disk-growth-by-project
audit: human_only_authorization
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

`human_only_authorization` in `crates/source-audit` parses every `.rs`
file under `crates/core/src`, `crates/cli/src`, and `crates/tui/src`
(skipping `crates/core/src/actions.rs` itself, where `approve`/
`add_standing_grant`/`revoke_grant` are defined and their internal
`write_grants` wiring is expected) and fails if any function other than
`cmd_approve`, `cmd_grant_add`, `cmd_grant_revoke`
(`crates/cli/src/main.rs`) or `execute_one`
(`crates/tui/src/actions.rs`) calls `actions::approve`,
`actions::add_standing_grant`, or `actions::revoke_grant`. Adding a new
call site anywhere else in the workspace fails the audit; adding one
inside the CLI's or TUI's existing allowed functions is expected and
passes.

Unit tests for both the accept and reject paths, plus a
self-check against the real repository, live in
`crates/source-audit/src/audits.rs` under
`human_only_authorization_tests`.
