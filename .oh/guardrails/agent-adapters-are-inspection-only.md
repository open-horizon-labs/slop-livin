---
id: agent-adapters-are-inspection-only
severity: hard
statement: "Identification never acts. An adapter declares an action capability on the unit it returns; only the shared sink -- with the plan, grant, recheck, ledger and Trash discipline -- executes anything. Adapters reference no action, plan, grant, ledger or filesystem-mutating API."
outcome: decision-relevant-storage-evidence
audit: agent_adapters_are_inspection_only
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

No adapter module may reference `actions::`, `fs::rename`,
`remove_file`, `remove_dir`, `trash::`, a `Plan {`/`Grant {` literal, or
`Ledger`.

**Limits.** Names, not semantics. An adapter shelling out to `rm` would
be caught by the repo's existing `scripts/check.sh` grep layer instead.

## Runtime tests that complete it

- `crates/core/tests/execution_rechecks.rs` — every destructive path in
  the test suite goes through the sink
