---
id: every-spawn-is-counted
severity: hard
statement: "Every subprocess swamp starts goes through `fs_gate::spawn::run`, which takes a `Program` (the allow-list), counts the spawn in `work_counters` before starting it, and returns only the finished output. No other production code names `std::process::Command`, so `subprocess_spawns == 0` is a measurement, not a habit."
outcome: decision-relevant-storage-evidence
audit: gate_paths_only_inside_gates
compile_fail:
  - spawn_takes_a_program_not_a_string
  - no_command_escapes_the_gate
runtime_tests:
  - crates/core/tests/reviewer_counterexamples_stack3.rs::every_command_new_must_record_a_spawn
  - crates/core/tests/reviewer_cost_measurement_stack3.rs
  - crates/core/tests/reviewer_cost_measurement_stack3.rs::spawn_oracle_covers_every_program_the_gate_can_run
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

Mechanism: type, gate audit, clippy, runtime test.

**Type.** `fs_gate::spawn::run` takes a `Program` (the enum is the allow-list), counts the spawn and returns a finished `RunOutput`; no `Command` leaves the gate.

**Gate audit.** `std::process` (other than `exit`/`id`/`ExitCode`) may be named only inside the gate -- in any spelling, `<std::process::Command>::new` included -- and each `Program` variant only in the modules that run it.

**Clippy.** `disallowed_types` rejects `std::process::Command` type-resolved outside the gate.

Compile-fail cases (`crates/core/tests/compile_fail/`, run by `crates/source-audit/tests/compile_fail.rs` against the production API): `spawn_takes_a_program_not_a_string`, `no_command_escapes_the_gate`.

## Runtime tests that complete it

- `spawn::tests::building_a_command_counts_one_spawn`
- the PATH-shim spawn oracle in `reviewer_cost_measurement_stack3.rs`
