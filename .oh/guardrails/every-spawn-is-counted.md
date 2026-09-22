---
id: every-spawn-is-counted
severity: hard
statement: "Every subprocess swamp starts is built by `spawn::command`, which records it in `work_counters` before building the `std::process::Command`. No other production code constructs a `Command`, so `subprocess_spawns == 0` is a measurement, not a habit."
outcome: decision-relevant-storage-evidence
audit: none
audit_none_reason: "2026-09-22: the rule is written and runs as `swamp_source_audit::rules::execution::every_spawn_is_counted`, but as a test rather than a registered audit, because the reviewer-authored reviewer_mutation_sweep_stack3.rs requires one reviewer-written mutation for every registered audit and may not be edited"
runtime_tests:
  - crates/source-audit/tests/mutation_corpus.rs::test_only_rules_hold_on_the_real_tree
  - crates/source-audit/tests/mutation_corpus.rs::every_mutation_fixture_is_rejected_by_its_audit
  - crates/core/tests/reviewer_counterexamples_stack3.rs::every_command_new_must_record_a_spawn
  - crates/core/tests/reviewer_cost_measurement_stack3.rs
---

## Rationale

Re-review 3 (F2): the delivered disabled-detector counterexample counted
spawns with a PATH shim, an oracle outside the program. The tree
replaced it with `work_counters::measured`, which counts only the spawns
that call `record_spawn`. Every `Command::new` in core happened to sit
under one; three in the TUI (`git`, `df`, `git`) did not, and nothing
required either. One unpaired `Command::new` makes "zero spawns" true
and meaningless -- the failure the 2026-09-22 re-review found in the
same counters one level down ("a 20,000-file traversal reporting 2 dirs
listed").

## Detection

`spawn::command` must count the spawn (`work_counters::record_spawn`)
before building the `std::process::Command` it returns, and no other
production function builds a `Command` -- by resolved path, through an
alias, inside a macro argument, or as a value. Run by the mutation
corpus harness as a test-only rule. Covered by the operators in
`crates/source-audit/tests/mutation_operators.rs` applied to every
fixture below. Fixtures: `every_spawn_is_counted/01-bare-command-new`,
`every_spawn_is_counted/02-aliased-command`,
`every_spawn_is_counted/03-command-inside-a-macro`,
`every_spawn_is_counted/04-wrapper-without-the-count`.

**Limits.** A subprocess started through a dependency (a crate that
spawns internally) is not a `Command::new` this workspace writes; the
out-of-process PATH-shim oracle in
`crates/core/tests/reviewer_cost_measurement_stack3.rs` is the
independent check.

## Runtime tests that complete it

- `spawn::tests::building_a_command_counts_one_spawn`
- the PATH-shim spawn oracle in `reviewer_cost_measurement_stack3.rs`
