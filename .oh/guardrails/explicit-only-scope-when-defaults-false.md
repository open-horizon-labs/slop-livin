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

Every definition of `detectors_permitted` reads `defaults` and `enabled_detectors`. Every function that takes the config type (derived: the `*Config` with `enabled_detectors`) and returns the effective scope, or produces roots by detector inference (reaching a `Detector` implementation) or by listing, must reach the predicate. The two named semantics tests are parsed, running and asserting.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `explicit_only_scope_when_defaults_false/01-reviewer-test-deleted`, `explicit_only_scope_when_defaults_false/02-second-resolver-without-the-guard`, `explicit_only_scope_when_defaults_false/03-permission-predicate-always-true`, `explicit_only_scope_when_defaults_false/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## Runtime tests that complete it

- `crates/core/tests/reviewer_counterexamples.rs::defaults_false_must_mean_explicit_only`
- `crates/core/src/scope.rs::tests::defaults_false_without_includes_or_enabled_detectors_is_empty`
- `crates/core/src/scope.rs::tests::defaults_false_with_an_enabled_detector_allow_list_runs_only_that_detector`
