---
id: folding-only-for-artifacts
severity: hard
statement: "The walk folds a directory into one sized unit only when it classifies as an artifact; every other directory is descended and gets its own rollup row."
outcome: disk-growth-by-project
audit: folding_only_for_artifacts
---

## Rationale
Folding is what keeps the walk affordable (a node_modules is one stat-tree, one row), and folding anything else would hide source directories from the growth-by-directory view.

## Detection
Every `AttrJob::Size` construction in `walk.rs` is inside an `if let Some(kind) = classify_at(..)` or inside `process_size` (recursion within an already-folded unit). AST audit `folding_only_for_artifacts`.
