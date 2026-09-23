---
id: agent-adapter-test-contract
severity: hard
statement: "Every adapter proves the same five things about itself, by these exact test names: unknown_format_is_explicit_not_empty, canary_content_never_appears_in_output, identification_reads_no_more_than_header_cap, protected_categories_default_protected, project_link_is_declared_or_unresolved_never_basename_guess."
outcome: decision-relevant-storage-evidence
audit: guardrail_metadata
runtime_tests:
  - crates/core/tests/agent_storage_validation.rs
---

## Rationale

The 2026-09-21 review's P1 on completion claims: the support matrix
labelled every tool "Supported" while the session notes admitted an
assumed Windsurf layout and unconfirmed Cline/Roo project fields. Test
*coverage* varied per adapter, so "Supported" meant different things in
different rows and nothing checked the difference.

Naming the five obligations makes the label mean one thing. An adapter
that cannot honestly write one of these tests is not Supported, and the
matrix has to say `Unverified`.

## Detection

Mechanism: gate audit, runtime test.

**Gate audit.** `guardrail_metadata` requires every adapter module that declares a `*_TOOL_ID` to hold the five contract tests in its own file, each a running `#[test]` (not ignored in any spelling, `#[cfg_attr(.., ignore)]` included) that invokes an assertion macro.

Retired 2026-09-22: the `agent_adapter_test_contract` source audit (a `syn` call-graph rule, which four review rounds showed cannot be made mutation-proof without type resolution; `docs/architecture.md`, "Capability gates"). Its mutation fixtures, and the sweep-3 and sweep-4 mutations aimed at it, now run in `crates/source-audit/tests/mutation_sweep.rs`, compiled: each must fail compilation (or clippy) or a gate audit.

## Runtime tests that complete it

- the five tests themselves, per adapter
- `crates/core/tests/agent_storage_validation.rs`
