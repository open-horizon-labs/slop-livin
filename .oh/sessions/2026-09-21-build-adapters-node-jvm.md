---
date: 2026-09-21
outcome: disk-growth-by-project
issues: [64, 65, 67, 68]
---

# Build adapters: the trait, the matrix, Node and the JVM

## The audits landed first, failing (GUARDRAILS_SPEC.md section 18)

Eight audits registered in `crates/source-audit/src/build_audits.rs`
before any adapter existed, so the baseline is the audit's own
enumeration rather than a prose list. The repo-level run
(`cargo run -p swamp-source-audit`) at commit 1:

```
FAIL  build_adapters_are_pluggable: crates/core/src/build_adapters/ has no adapter modules
FAIL  build_adapters_are_inspection_only: (same)
FAIL  build_adapters_read_bounded_manifests_only: (same)
FAIL  build_adapters_do_not_traverse: (same)
FAIL  build_units_built_through_builder: (same)
FAIL  build_adapter_test_contract: (same)
FAIL  build_adapter_matrix_matches_docs: (same)
FAIL  build_adapters_reuse_under_event_coverage: read crates/core/src/build_adapters/mod.rs:
      No such file or directory
8 audit(s) failed
```

Every one of the eight carries three rejection fixtures in the mutation
corpus, including an alias/rename variant and, where the audit is about a
result rather than a call, a discarded-result variant --
`NOT_YET_IN_THE_CORPUS` stays empty.

A deliberate choice in the audit shape: `adapters_or_err` makes an
*absent* `build_adapters/` directory the first failure of seven of the
eight rules, rather than letting them pass vacuously over an empty
directory. An audit that passes because the thing it governs does not
exist is the failure mode section 18 exists to prevent.
