---
id: discovery-consumes-effective-scope
severity: hard
statement: "Only crate::scope interprets detector candidates. Discovery and measurement consume EffectiveScope::authorized_roots(), which has already applied exclusions, disabled detectors and explicit-root replacement. An excluded home yields zero units; a disabled detector yields zero units; a missing or unreadable root yields a coverage note and zero units, never a tombstone."
outcome: coverage-aware-storage-history
audit: discovery_consumes_effective_scope
---

## Rationale

`excluded_agent_home_must_not_be_scanned` in the 2026-09-21 review:
excluding the entire Claude Code home still produced one agent unit and
one external unit. Both `external::discover_and_measure` and
`agents::discover_and_measure` iterated `scope.detectors`' raw
`Resolved` candidates, which is detector *output*, not authorized
scope — so no exclusion, no disabled detector and no explicit-root
replacement could reach them. The CLI and TUI compounded it by
re-resolving the scope with empty explicit roots, widening commands the
user had deliberately narrowed.

An exclusion the tool accepts and then ignores is worse than an
exclusion it refuses.

## Detection

The discovery region is derived: the modules (and child modules) exactly reachable from `external::discover_and_measure` and `agents::discover_and_measure`, plus the adapters, less the modules that define or produce raw detector output. No function there reads a field that holds raw detector output wholesale (derived from the field types: collections of `DetectorSummary`/`ProposedLocation`), whatever the binding is called, or names a `LocationStatus` variant; both discovery passes reach `EffectiveScope::authorized_roots()`.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `discovery_consumes_effective_scope/01-raw-detector-candidates`, `discovery_consumes_effective_scope/02-resolved-status-in-an-adapter`, `discovery_consumes_effective_scope/03-summary-locations-one-file-away`, `discovery_consumes_effective_scope/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## Runtime tests that complete it

- `crates/core/tests/reviewer_counterexamples.rs::excluded_agent_home_must_not_be_scanned`
- `crates/core/src/scope.rs::tests::authorized_roots_drops_excluded_and_explains_missing`
- `crates/core/src/scope.rs::tests::explicit_roots_do_not_authorize_detector_paths_outside_them`
- `crates/core/tests/reviewer_counterexamples_stack2.rs::an_excluded_home_must_stay_excluded_under_an_explicit_root`
  — the CE1 case the prior test could not see, because it passed `&[]`
  for explicit roots.
- `crates/core/tests/explicit_root_scope_exclusions.rs` — every
  exclusion case (an excluded home, an excluded nested location, a
  disabled detector, a protected path in either spelling) under an
  explicit root that *contains* it.
- `crates/cli/tests/` — `report <root> --view agents --json` on a
  fixture root that does not contain the agent home returns zero units
  with a coverage note saying why.
