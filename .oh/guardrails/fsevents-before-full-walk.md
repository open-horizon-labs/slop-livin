---
id: fsevents-before-full-walk
severity: hard
statement: "An observation asks its platform's change-observation source what changed before it walks anything; a full walk happens only when the replay refuses, the store has no anchor, --full is passed, or the classification rules version changed. Where the platform has no persisted change history, the refusal names that rather than reporting a missing backend."
outcome: disk-growth-by-project
audit: fsevents_before_full_walk
---

## Rationale
The whole point of persisting an event id is that the next observation is a replay plus a re-walk of the implicated subtrees, not a fresh walk. On ~/src (41 GB, 107 worktrees) a full walk is ~7 s; the replay is ~0.1 s.

## Detection
In `growth::observe_tracked_with_source`, the call to `replay` precedes every `full_walk` call that is not guarded by `force_full` or `rules_changed`. AST audit `fsevents_before_full_walk`. The ordering rule is the same on both platforms -- only which source answers differs. Which refusal a platform without replay gives is decided in one place, from `platform::ContinuitySource`, and audit `platform_capabilities_gate_their_backends` fails a second copy of that decision.

## Limits
On Linux the replay always refuses (`no_persisted_change_history`), so the ordering is satisfied trivially there and this guardrail buys nothing until #81. It is still the right shape: it is what makes #81 a new source rather than a new code path.
