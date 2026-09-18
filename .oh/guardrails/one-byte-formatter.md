---
id: one-byte-formatter
severity: hard
statement: "There is one byte formatter in the product, decimal, SI-labelled; the TUI re-exports core's."
outcome: disk-growth-by-project
audit: one_byte_formatter
---

## Rationale
The TUI once divided by 1024 under a GB label while core divided by 1000; rows visibly failed to sum. AST audit `one_byte_formatter`.
