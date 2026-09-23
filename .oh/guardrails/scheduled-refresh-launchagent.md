---
id: scheduled-refresh-launchagent
severity: hard
statement: "A scheduled observation exists as an opt-in per-user LaunchAgent so a previous observation is there without a human running anything."
outcome: disk-growth-by-project
audit: gate_paths_only_inside_gates
compile_fail:
  - launch_agent_is_a_named_text_file
runtime_tests:
  - crates/core/src/schedule.rs::tests::plist_golden_content
  - crates/core/src/schedule.rs::tests::off_removes_plist_and_issues_bootout_in_test_mode
---

## Rationale
Growth needs a baseline. Without a schedule the first question a user asks has no history behind it.

## Detection

Mechanism: type, gate audit, runtime test.

**Type.** The plist is written only as `TextFile::LaunchAgent` (validated to `<label>.plist`).

**Gate audit.** `Program::Launchctl` may be named only in `schedule`.

Retired 2026-09-22: the `scheduled_refresh_launchagent` source audit (a `syn` call-graph rule, which four review rounds showed cannot be made mutation-proof without type resolution; `docs/architecture.md`, "Capability gates"). Its mutation fixtures, and the sweep-3 and sweep-4 mutations aimed at it, now run in `crates/source-audit/tests/mutation_sweep.rs`, compiled: each must fail compilation (or clippy) or a gate audit.

Compile-fail cases (`crates/core/tests/compile_fail/`, run by `crates/source-audit/tests/compile_fail.rs` against the production API): `launch_agent_is_a_named_text_file`.
