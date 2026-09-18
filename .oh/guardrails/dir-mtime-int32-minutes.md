---
id: dir-mtime-int32-minutes
severity: hard
statement: "Per-directory and per-file store rows carry the newest mtime as int32 minutes (mod_time_min), the store-depth design from the Go plumbing."
outcome: disk-growth-by-project
audit: dir_mtime_int32_minutes
---

## Detection
`growth::dirs_schema` and `files_schema` declare a `mod_time_min` Int32 field. AST audit `dir_mtime_int32_minutes`.
