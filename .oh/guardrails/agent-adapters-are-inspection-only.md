---
id: agent-adapters-are-inspection-only
severity: hard
statement: "Identification never acts. An adapter declares an action capability on the unit it returns; only the shared sink -- with the plan, grant, recheck, ledger and Trash discipline -- executes anything. Adapters reference no action, plan, grant, ledger or filesystem-mutating API."
outcome: decision-relevant-storage-evidence
audit: adapters_do_not_reach_gates, gate_paths_only_inside_gates
compile_fail:
  - trash_move_needs_a_recheck_proof
  - recheck_proof_is_minted_only_by_run_all
runtime_tests:
  - crates/core/tests/execution_rechecks.rs
---

## Rationale

"Inspection is not authorization" is a handoff constraint, and the whole
safety argument for agent storage rests on a single sink where identity,
protection and occupancy are rechecked. Fourteen adapters, each free to
call `fs::rename`, would be fourteen places that argument has to be
re-made. The review's counterexamples were all failures *at* the sink;
they would have been unrecoverable if the sink were not the only way
through.

## Detection

Mechanism: type, gate audit, runtime test.

**Type.** Every destructive operation lives in `fs_gate::destroy` and takes a `RecheckProof` and an `Authorized`; an adapter has neither.

**Gate audit.** `adapters_do_not_reach_gates` limits adapters to `IdentifyCtx`; `gate_paths_only_inside_gates` rejects `std::fs`, `libc` (`truncate`, `unlink`), `trash` and `std::process` outside the gate, and `fs_gate::destroy` outside the execution sinks.

Retired 2026-09-22: the `agent_adapters_are_inspection_only` source audit (a `syn` call-graph rule, which four review rounds showed cannot be made mutation-proof without type resolution; `docs/architecture.md`, "Capability gates"). Its mutation fixtures, and the sweep-3 and sweep-4 mutations aimed at it, now run in `crates/source-audit/tests/mutation_sweep.rs`, compiled: each must fail compilation (or clippy) or a gate audit.

Compile-fail cases (`crates/core/tests/compile_fail/`, run by `crates/source-audit/tests/compile_fail.rs` against the production API): `trash_move_needs_a_recheck_proof`, `recheck_proof_is_minted_only_by_run_all`.

## Runtime tests that complete it

- `crates/core/tests/execution_rechecks.rs` — every destructive path in
  the test suite goes through the sink
