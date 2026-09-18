---
id: walk-optimized-parallel-pool
severity: soft
statement: "The full walk runs on the work-stealing pool with folded units sized as parallel Size jobs; no serial walk on the report path."
outcome: disk-growth-by-project
audit: walk_optimized_parallel_pool
---

## Rationale
15 s → 7.5 s on ~/src came from the pool and from sizing folded units in parallel. The serial `attribution::attribute` walk exists for tests only.

## Detection
`walk::discover_and_attribute` calls `attribute_parallel`; nothing under `report.rs` or `consumers/` calls `attribution::attribute(`. AST audit `walk_optimized_parallel_pool`.
