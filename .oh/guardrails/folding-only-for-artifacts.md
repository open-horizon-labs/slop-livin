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
AST audit `folding_only_for_artifacts`:
1. every `AttrJob::Size` literal in `walk.rs` sits in the then-branch of an `if let ... = classify_at(..)` (or inside `process_size`, recursion within a folded unit, or `resize_artifact_with_dirs`, which re-sizes a path the store already classified);
2. every `record_artifact` call in the serial walker sits under the same guard;
3. `classify_at` ends in a table lookup or `None` and, like `classify` and `classify_gated`, constructs no `ArtifactKind` of its own.

Proven by `scripts/audit-mutants.sh`: folding under a made-up kind, recording without classifying, and making `classify_at` default to `Cache` each fail this audit.
