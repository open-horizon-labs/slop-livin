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

Every `AttrJob::Size` construction and every `record_artifact` call anywhere must sit under an `if let .. = classify_at(..)` guard, except in the measured folding entry points listed by resolved path (`walk::process_size`, `walk::resize_artifact_stamped`), each of which must be defined beside `AttrJob`. A fold or record written inside a macro argument fails. `classify_at` ends in a table lookup or `None`; the classifiers invent no kind.

Covered by the operators in `crates/source-audit/tests/mutation_operators.rs` (alias, pub-use shim, same-file helper, child module, macro wrap, constant hoisting, injection into an exempt bounded primitive; discard, and precision variants, for legitimate seeds), applied to every fixture below. Fixtures: `folding_only_for_artifacts/01-fold-without-classify`, `folding_only_for_artifacts/02-fold-under-a-different-guard`, `folding_only_for_artifacts/03-serial-walk-records-without-classify`, `folding_only_for_artifacts/04-sweep3`.

**Limits.** The program model (`crates/source-audit/src/program.rs`) is lexical: a method call on a receiver whose type it cannot see is possibly every method of that name and arity; trait-object dispatch resolves to every implementor; a function pointer stored in a struct and a `proc_macro` that generates calls are invisible.

