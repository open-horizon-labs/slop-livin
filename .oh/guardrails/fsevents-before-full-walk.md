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
In `growth::observe_tracked_with_source`, the call to `replay` precedes every `full_walk` call that is not guarded by `force_full` or `rules_changed`. AST audit `fsevents_before_full_walk`.
