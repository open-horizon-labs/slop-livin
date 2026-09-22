---
id: protection-fails-closed
severity: hard
statement: "Human keep/protect intent is loaded through one function that returns a Result; unreadable or malformed protection state is unknown, never an empty keep list, and every action refuses until it can be read. Protection is tested in both directions: a unit beneath a protected path and a unit containing one. The list is written atomically."
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

## Detection

Three checks in `crates/source-audit`:

1. `agents::protection_conflict` (or `is_human_protected`) must contain
   both containment tests — `candidate.starts_with(p)` and
   `p.starts_with(candidate)`. Removing either direction fails.
2. `protect_add` and `protect_remove` must reach a function named
   `write_atomic` (temp file + rename), following calls transitively
   within `agents/mod.rs`.
3. No function anywhere in `crates/{core,cli,tui}/src` may follow a
   `load_protect(`/`protect_list(` call with `.unwrap_or_default()`,
   `.unwrap_or(`, `.unwrap_or_else(` or `.ok()` within the next 120
   tokens.

**Limits.** Check 3 is a bounded textual window after the call, so an
error discarded several statements later through an intermediate binding
is not caught; `scripts/check.sh` adds a grep layer for
`unwrap_or_default()` on any line mentioning `protect`.

## Runtime tests that complete it

- `crates/core/tests/reviewer_counterexamples.rs::protected_descendant_must_prevent_parent_cache_proposal`
- `crates/core/tests/execution_rechecks.rs` — corrupt protect file
  refuses every action and `swamp protect list` reports the corruption
  rather than an empty list; protection added on an ancestor and on a
  descendant after approval both refuse.
