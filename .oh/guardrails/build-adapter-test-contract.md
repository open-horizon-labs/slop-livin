---
id: build-adapter-test-contract
severity: hard
statement: "Every build adapter proves the same five things about itself, in tests named exactly `unknown_layout_is_explicit_not_empty`, `identification_reads_no_more_than_manifest_cap`, `no_project_or_build_code_is_executed`, `variants_never_collapse_by_basename` and `age_is_not_obsolescence`, none ignored and none assertion-free."
outcome: disk-growth-by-project
audit: build_adapter_test_contract
---

## Rationale

The five names are not a checklist; each one is a specific way the
#64-#68 issues say this work fails.

- *unknown layout is explicit, not empty* -- "not one generic
  node_modules/dist row and not an unimplemented unknown for every tool"
  (#68), and "unsupported layouts yield a named limitation" (#66). An
  adapter that returns `vec![]` for a layout it does not understand is
  indistinguishable from an empty directory.
- *reads no more than the manifest cap* -- the per-refresh cost has to be
  a measured number, not a claim.
- *no project or build code is executed* -- the runtime complement to the
  inspection-only audit.
- *variants never collapse by basename* -- "a filename-only hash grouper
  fails acceptance" (#66); two workspaces' `dist`, two target triples'
  `debug`, two Gradle configurations' `build/classes`.
- *age is not obsolescence* -- "old is not unused"; modification age may
  suggest review and may never be recorded as a verdict.

## Detection

Each adapter module must define all five, each not `#[ignore]`d and each
containing at least one `assert!`/`assert_eq!`/`assert_ne!`/`contract::`
call. Section 17 item 4: an existence-only check is satisfied by a name.

**Limits.** The audit cannot tell a strong assertion from
`assert!(true)`. It guarantees the test runs and asserts *something*;
review and the shared contract tests in
`crates/core/tests/build_adapter_contract.rs` cover the rest.

## Runtime tests that complete it

- the five named tests in each of `build_adapters/{cargo,node,gradle,maven}.rs`
