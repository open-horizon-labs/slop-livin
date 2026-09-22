---
id: protection-fails-closed
severity: hard
statement: "Human keep/protect intent is loaded through one function that returns a Result; unreadable or malformed protection state is unknown, never an empty keep list, and every action refuses until it can be read. There is exactly ONE protection predicate (agents::protection_conflict), it tests both directions -- a unit beneath a protected path and a unit containing one -- and no other function answers the question except by delegating to it. The list is written atomically."
outcome: decision-relevant-storage-evidence
audit: protection_fails_closed
---

## Rationale

Two findings from the 2026-09-21 review:

- `protected_descendant_must_prevent_parent_cache_proposal` —
  `is_human_protected` only asked whether the candidate lay beneath a
  protected path. Protecting `debug/log.txt` therefore did nothing to
  stop `debug/` being proposed and removed, destroying exactly what the
  human asked to keep.
- The review's closing note: a malformed `agent_protect.json` was parsed
  with `unwrap_or_default()` into an empty list, silently *lifting*
  every protection, and writes were non-atomic so a crash mid-write
  could produce that malformed file in the first place.

A protection mechanism that fails open is worse than none: it reports
that intent was recorded and then ignores it.

A third finding, from the integration owner's own mutation check on
2026-09-21, is why this guardrail now insists on *one* predicate.
Mutating protection to a single direction and running everything
**passed** -- this audit and every runtime test. The reason: the audit
inspected `protection_conflict`, while
`actions::propose_checking_protection` (the path for ordinary
filesystem rows) went through a second predicate,
`agents::is_human_protected`, a `bool` wrapper that *looked* like a
delegation. No test proposed an ordinary directory that contained a
protected descendant, so the surviving direction was never exercised
where it mattered.

Two spellings of one question means two things to inspect, and an audit
will always be reading the other one. `is_human_protected` is therefore
deleted rather than fixed, and the audit forbids reintroducing it under
any name.

## Detection

`agents::protection_conflict` tests containment in both directions; every function that persists the protect list goes through `agents::write_atomic`; every function that loads the list and does not manage it reaches the predicate; no loader's error is turned into an empty list (resolved, so an alias is still the loader); in modules that define an execution or proposal entry point, a verdict-returning path-containment test must be, or reach, the one predicate.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `protection_fails_closed/01-one-directional-helper`, `protection_fails_closed/02-discarded-error`, `protection_fails_closed/03-aliased-discard`, `protection_fails_closed/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

## Runtime tests that complete it

- `crates/core/tests/reviewer_counterexamples.rs::protected_descendant_must_prevent_parent_cache_proposal`
- `crates/core/tests/execution_rechecks.rs` — corrupt protect file
  refuses every action and `swamp protect list` reports the corruption
  rather than an empty list; protection added on an ancestor and on a
  descendant after approval both refuse.
- `crates/core/tests/evidence_action_recheck.rs::ordinary_unit_containing_protected_descendant_is_refused_at_proposal`
  and its sibling `..._beneath_a_protected_ancestor_...` — the exact
  case the mutation escaped, on the **ordinary** filesystem row path, at
  proposal time. Both directions, in the same file, so a future
  one-directional mutation fails whichever half it drops.
- `crates/tui/tests/scope_preserving_refresh.rs::marking_an_ordinary_row_that_contains_a_protected_file_is_refused_with_the_reason`
  — the same case on the TUI's mark path, which now proposes through
  `propose_checking_protection` so the refusal arrives when the human
  marks the row rather than at execution.
