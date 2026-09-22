---
id: one-byte-formatter
severity: hard
statement: "There is one byte formatter in the product, decimal, SI-labelled; the TUI re-exports core's."
outcome: disk-growth-by-project
audit: one_byte_formatter
---

## Rationale
The TUI once divided by 1024 under a GB label while core divided by 1000; rows visibly failed to sum. AST audit `one_byte_formatter`.

## Detection

`render::human_bytes` divides by 1000; no other definition carries the formatter's name; the TUI re-exports core's formatter; and no function anywhere divides by 1024 and puts a binary unit label into formatted output, whether the label is in the macro or in a constant (a unit table) it indexes.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `one_byte_formatter/01-second-1024-formatter`, `one_byte_formatter/02-core-formatter-uses-1024`, `one_byte_formatter/03-tui-defines-its-own`, `one_byte_formatter/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.
