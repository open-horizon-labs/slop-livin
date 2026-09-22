---
id: symlinks-never-followed
severity: hard
statement: "The walk never follows a symlink: every directory listing that descends checks is_symlink first and discards the entry."
outcome: disk-growth-by-project
audit: symlinks_never_followed
---

## Rationale
Following symlinks double-counts bytes and can loop. Symlinks are either optimized (counted once by inode) or discarded; they are never traversed.

## Detection

Anywhere: no size is taken from a following `fs::metadata`, and nothing canonicalizes a child (`.join(..)`) path. In every function that lists a directory: no following stat of a listed entry, and no `is_dir`/`is_file`/`exists` on a `.path()` or a `.join(..)`.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `symlinks_never_followed/01-aliased-metadata`, `symlinks_never_followed/02-plain-metadata`, `symlinks_never_followed/03-canonicalize-a-child`, `symlinks_never_followed/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

