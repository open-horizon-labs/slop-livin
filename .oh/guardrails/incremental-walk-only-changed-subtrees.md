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

Every `apply_incremental` calls `attribute_one_worktree` and a `resize_artifact*`, and any other call it makes that reaches the traversal set must be handed the changed paths (an argument derived, through bindings and collections they are pushed into, from its changed-path parameter).

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `incremental_walk_only_changed_subtrees/01-full-walk-fallback`, `incremental_walk_only_changed_subtrees/02-parallel-whole-tree`, `incremental_walk_only_changed_subtrees/03-no-targeted-rewalk`, `incremental_walk_only_changed_subtrees/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

