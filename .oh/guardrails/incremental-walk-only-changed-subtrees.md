---
id: incremental-walk-only-changed-subtrees
severity: hard
statement: "The incremental path re-walks only the worktrees and folded artifacts FSEvents implicated and carries every other row forward from the store; it never calls the full parallel walk."
outcome: disk-growth-by-project
audit: incremental_walk_only_changed_subtrees
---

## Rationale
Incremental means incremental. If the incremental path can fall into a full walk for anything but a refusal, the store's anchor is decoration.

## Detection
`growth::apply_incremental` calls `attribute_one_worktree` / `resize_artifact` and never `full_walk` or `attribute_parallel`. AST audit `incremental_walk_only_changed_subtrees`.
