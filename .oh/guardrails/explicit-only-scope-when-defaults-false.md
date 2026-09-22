---
id: explicit-only-scope-when-defaults-false
severity: hard
statement: "defaults = false means explicit-only scope: swamp infers nothing. In scope are the include roots, explicit command roots, and detectors the config names (enabled_detectors, or the complement of a non-empty disabled_detectors). With neither list set, the scope is empty and the command says so rather than falling back to cwd or home."
outcome: coverage-aware-storage-history
audit: explicit_only_scope_when_defaults_false
---

## Rationale

`defaults_false_must_mean_explicit_only`: with `defaults = false` and no
include, scope resolution still inferred 111 candidate roots, because
the flag disabled only the builtin-defaults detector. The
implementation recorded that as a documented judgment call and relabelled
the result "explicit-only".

The review's finding stands: a documented judgment call is not user
approval to change an explicit scope contract. The user has since
rejected the earlier reading outright (dated correction in
`.oh/sessions/2026-09-21-scope-and-detector-registry.md`).

## Detection

`ScanConfig` must have an `enabled_detectors` field; `scope.rs` must
define `detectors_permitted(config)`; and `resolve_effective_scope` must
call it to guard detector inference.

**Limits.** Semantics cannot be proved from the AST — a
`detectors_permitted` that always returned `true` would pass the
structural half. The audit therefore also requires two tests to exist by
name, and CI runs them:
`crates/core/src/scope.rs::tests::defaults_false_without_includes_or_enabled_detectors_is_empty`
and the reviewer's `defaults_false_must_mean_explicit_only`.

## Runtime tests that complete it

- `crates/core/tests/reviewer_counterexamples.rs::defaults_false_must_mean_explicit_only`
- `crates/core/src/scope.rs::tests::defaults_false_without_includes_or_enabled_detectors_is_empty`
- `crates/core/src/scope.rs::tests::defaults_false_with_an_enabled_detector_allow_list_runs_only_that_detector`
