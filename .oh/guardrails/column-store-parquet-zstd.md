---
id: column-store-parquet-zstd
severity: hard
statement: "Every persisted observation is Parquet written with zstd compression, keyed per volume; no JSON, SQLite or bespoke binary store for rows."
outcome: disk-growth-by-project
audit: column_store_parquet_zstd
---

## Rationale
Column store + reverse delta was the design carried from the Go plumbing. It is what makes growth over any window a lookup and keeps the store small enough to keep for months.

## Detection
Every function in `growth.rs` that constructs an `ArrowWriter` declares ZSTD compression in the same function (`Compression::ZSTD` or `zstd_properties`). AST audit `column_store_parquet_zstd`.
