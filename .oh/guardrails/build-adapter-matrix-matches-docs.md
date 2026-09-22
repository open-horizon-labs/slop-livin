---
id: build-adapter-matrix-matches-docs
severity: hard
statement: "`docs/build-artifacts.md`'s capability matrix has exactly one row per registered adapter, every row is also an entry in `build_adapters::matrix`, and every row states inspection only for as long as no adapter implements an action."
outcome: disk-growth-by-project
audit: build_adapter_matrix_matches_docs
---

## Rationale

The agent-side version of this rule caught a published support table
promising actions the code refused. Here the same drift would be worse:
this epic's whole shape is identification delivered *without* cleanup
(#73 implements adapter actions later), so a docs row reading anything
but "inspection only" is a promise nothing in the codebase can keep, and
a user reading the table would plan around it.

A capability matrix that is not checked is a marketing document. #64 asks
for "a checked coverage matrix" and #68 for "a checked per-tool
capability matrix"; checked means an executable comparison, in both
directions.

## Detection

For every registered adapter: `docs/build-artifacts.md` contains a row
naming its id in backticks, and `matrix.rs` contains an entry for it.
Every table row naming an adapter must contain "inspection only".

**Limits.** The audit matches rows by adapter id and checks the action
column's wording. Role and layout columns are compared against the code
by the executable test below, which parses the table rather than
grepping it.

## Runtime tests that complete it

- `crates/core/tests/build_matrix_docs.rs::docs_table_equals_the_capability_matrix`
