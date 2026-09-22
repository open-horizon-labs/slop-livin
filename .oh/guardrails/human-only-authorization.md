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

No production function outside the sinks' own module reaches `actions::approve`, `add_standing_grant` or `revoke_grant` -- by resolved call, re-export (including a local shim module) or value reference -- except the reviewed callers, each of which must still exist.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `human_only_authorization/01-aliased-grant-mint`, `human_only_authorization/02-unlisted-caller`, `human_only_authorization/03-moved-one-file-away`, `human_only_authorization/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

