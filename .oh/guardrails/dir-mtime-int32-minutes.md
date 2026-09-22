---
id: dir-mtime-int32-minutes
severity: hard
statement: "Per-directory and per-file store rows carry the newest mtime as int32 minutes (mod_time_min), the store-depth design from the Go plumbing."
outcome: disk-growth-by-project
audit: dir_mtime_int32_minutes
---

## Detection

Every `Field::new` anywhere whose name (a literal or a constant) starts with `mod_time` must be `mod_time_min` with `DataType::Int32`; `dirs_schema` and `files_schema` exist, and at least two schemas declare the minutes column.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `dir_mtime_int32_minutes/01-seconds-in-the-minutes-column`, `dir_mtime_int32_minutes/02-renamed-column`, `dir_mtime_int32_minutes/03-widened-column`, `dir_mtime_int32_minutes/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

