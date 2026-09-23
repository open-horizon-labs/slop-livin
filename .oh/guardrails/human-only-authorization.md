---
id: human-only-authorization
severity: hard
statement: "Authorization (approve a plan, add or revoke a grant, change the keep list) is minted only from the reviewed confirmation handlers -- the CLI's `cmd_approve`, `cmd_grant_add` and `cmd_protect`, and the TUI's `start_delete` -- each confirmation naming exactly what the human was shown (a plan's id and content digest, standing-grant terms, one keep-list change, one TUI unit) and spent once; and a plan or grant is authorization only as swamp's own propose/approve path wrote it (a keyed binding under the store's authority key, checked by the loader). This is a code-review boundary, not a human-identity boundary: any shell-capable process, including an agent, can invoke the CLI subcommands that reach those call sites. The sink (grant scope/budget/expiry checks, per-unit re-derivation at execute time, the ledger) is what actually limits what an authorized action can do, independent of who or what invoked it."
outcome: disk-growth-by-project
audit: gate_paths_only_inside_gates
compile_fail:
  - authorized_is_minted_only_by_authorize
  - human_confirmation_is_not_a_struct_literal
  - approval_without_a_confirmation_does_not_exist
  - plan_is_not_a_struct_literal
  - plan_is_not_deserialize
  - plan_content_cannot_be_edited
  - grant_is_not_a_struct_literal
  - grant_is_not_deserialize
  - authorize_is_not_callable_outside_core
  - human_confirmation_is_spent_once
  - human_confirmation_names_what_was_confirmed
  - authorize_confirmed_spends_the_confirmation
runtime_tests:
  - crates/core/tests/execution_rechecks.rs
  - crates/core/tests/actions_r7.rs
  - crates/core/tests/token_binding.rs
  - crates/core/tests/token_binding.rs::a_plan_edited_after_approval_does_not_execute
  - crates/core/tests/token_binding.rs::a_hand_edited_grant_refuses_every_execution
  - crates/core/tests/token_binding.rs::a_confirmation_for_one_plan_does_not_approve_another
  - crates/core/tests/token_binding.rs::a_tui_confirmation_refuses_a_path_it_does_not_name
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

**Type.** Approving a plan (`actions::approve_confirmed`), minting a standing grant (`add_standing_grant_confirmed`) and changing the keep list (`protect_{add,remove}_confirmed`) take a `HumanConfirmed` **by value**: it is not `Clone`, so it is spent once. Each constructor binds a subject -- `cli_approve(actor, &plan)` the plan's id and `Plan::content_digest`, `cli_grant` the typed terms, `cli_protect` one change, `tui_dialog` one confirmation per listed item -- and each consumer refuses a confirmation whose subject or site is not exactly what it is about to do. A one-shot grant records the plan's content digest and the confirmation's id; `authorize` (crate-private) refuses a plan whose content no longer has that digest. `Plan` and `Grant` have private fields and are not `Deserialize`: they are built by the propose/approve paths or by `load_plan`/`list_grants`, which check each record's keyed binding under the store's authority key (`fs_gate::key`) and refuse an edited, copied or hand-written file. Destructive calls take an `Authorized` from `authority::authorize` (a verified grant) or `authorize_confirmed` (one TUI confirmation, for the one path it names).

**Gate audit.** `gate_paths_only_inside_gates` pins each constructor to one function by resolved path: `cli_approve` to the CLI's `cmd_approve`, `cli_grant` to `cmd_grant_add`, `cli_protect` to `cmd_protect`, `tui_dialog` to the TUI app's `start_delete`, `authorize` to `actions::execute_with_trash_opts`, `authorize_confirmed` to the TUI's `execute_one`; `actions::revoke_grant` only in the CLI; the authority key only in `actions`, `authority` and `recheck`. It also pins the struct literals of `Plan`, `PlanUnit`, `Grant`, `Authorized`, `HumanConfirmed`, `RecheckProof` and `Trashed` to their constructor functions, which privacy alone cannot do inside the defining module.

**Limit.** The authority key is a file the user swamp runs as can read; a process with that access that reimplements the MAC can forge a record. What the binding rules out is every path that does not go through swamp's own code (`docs/architecture.md`, "Capability gates").

Retired 2026-09-22: the `human_only_authorization` source audit (a `syn` call-graph rule, which four review rounds showed cannot be made mutation-proof without type resolution; `docs/architecture.md`, "Capability gates"). Its mutation fixtures, and the sweep-3 and sweep-4 mutations aimed at it, now run in `crates/source-audit/tests/mutation_sweep.rs`, compiled: each must fail compilation (or clippy) or a gate audit.

Compile-fail cases (`crates/core/tests/compile_fail/`, run by `crates/source-audit/tests/compile_fail.rs` against the production API): `authorized_is_minted_only_by_authorize`, `human_confirmation_is_not_a_struct_literal`, `approval_without_a_confirmation_does_not_exist`, and (re-review 5) `plan_is_not_a_struct_literal`, `plan_is_not_deserialize`, `plan_content_cannot_be_edited`, `grant_is_not_a_struct_literal`, `grant_is_not_deserialize`, `authorize_is_not_callable_outside_core`, `human_confirmation_is_spent_once`, `human_confirmation_names_what_was_confirmed`, `authorize_confirmed_spends_the_confirmation`. Mutation fixtures: `crates/source-audit/tests/mutations/token_binding_and_gate_hardening/`.
