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

Every `ArrowWriter::try_new`/`new` call anywhere in the workspace must pass writer properties (not `None`), and the writing function, or a helper it calls, must declare `Compression::ZSTD`.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `column_store_parquet_zstd/01-uncompressed-writer`, `column_store_parquet_zstd/02-snappy-instead-of-zstd`, `column_store_parquet_zstd/03-uncompressed-writer-in-the-other-store-file`, `column_store_parquet_zstd/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

