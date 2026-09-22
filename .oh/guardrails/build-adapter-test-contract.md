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

All eight build-adapter rules range over **derived** sets
(`crates/source-audit/src/build_audits.rs`, module doc): the governed
modules are every file under `crates/core/src/build_adapters/` --
`mod.rs`, `registry.rs`, `matrix.rs`, `jvm_common.rs` and `bounded_io.rs`
included, no file exempt by name -- plus any workspace file holding an
`impl BuildAdapter for ..`; adapters, their types and their ids are read
from those impls. Re-review 3 (`review/REVIEW-STACK-3.md` section 1)
found 31 of 43 audit slips were a hand-written list that did not contain
the thing; these rules keep no such list except the four bounded
primitives, each of which is itself checked to name its cap.

For every derived adapter (anywhere in the workspace), the file defines
the five functions `unknown_layout_is_explicit_not_empty`,
`identification_reads_no_more_than_manifest_cap`,
`no_project_or_build_code_is_executed`,
`variants_never_collapse_by_basename` and `age_is_not_obsolescence` as
real `fn` items (a name in a comment does not count), inside a
`#[cfg(test)]` module, with `#[test]`, not `#[ignore]`d, and containing
an `assert!`/`assert_eq!`/`assert_ne!` or a `contract::` call.

**Limits.** "Contains an assertion" is structural: an assertion of a
tautology passes. The assertions' substance is reviewed, and the shared
runtime contract is `crates/core/tests/build_adapter_contract.rs`.

## Runtime tests that complete it

- the five named tests in each of `build_adapters/{cargo,node,gradle,maven}.rs`
