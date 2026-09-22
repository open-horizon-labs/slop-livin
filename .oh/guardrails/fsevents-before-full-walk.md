---
id: fsevents-before-full-walk
severity: hard
statement: "An observation asks FSEvents what changed before it walks anything; a full walk happens only when the replay refuses, the store has no anchor, --full is passed, or the classification rules version changed."
outcome: disk-growth-by-project
audit: fsevents_before_full_walk
---

## Rationale
The whole point of persisting an event id is that the next observation is a replay plus a re-walk of the implicated subtrees, not a fresh walk. On ~/src (41 GB, 107 worktrees) a full walk is ~7 s; the replay is ~0.1 s.

## Detection

Every `observe_tracked_with_source` delegates, honoured, to `stage_tracked_with_source` and reaches no walk except through it. In every `stage_tracked_with_source`, no call before the first FSEvents `replay*` reaches the derived traversal set unless it is under the `force_full` condition.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `fsevents_before_full_walk/01-force-full-always`, `fsevents_before_full_walk/02-aliased-full-walk`, `fsevents_before_full_walk/03-no-cursor-check`, `fsevents_before_full_walk/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

