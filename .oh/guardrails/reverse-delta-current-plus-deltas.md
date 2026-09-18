---
id: reverse-delta-current-plus-deltas
severity: hard
statement: "The store holds one current-state file plus a reverse-delta log; an observation rewrites current and appends the previous values of changed rows as a delta, for artifacts, directories and files alike."
outcome: disk-growth-by-project
audit: reverse_delta_current_plus_deltas
---

## Rationale
Reverse deltas make the latest state a single read and history a replay backwards, which is what a sparkline or a growth window needs. The DuckDB→Go port once discarded this design; it does not get discarded again.

## Detection
`observe_and_annotate`, `observe_and_annotate_dirs` and `observe_and_annotate_files` each write both the current path and a delta path. AST audit `reverse_delta_current_plus_deltas`.
