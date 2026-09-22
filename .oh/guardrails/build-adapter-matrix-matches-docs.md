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

All eight build-adapter rules range over **derived** sets
(`crates/source-audit/src/build_audits.rs`, module doc): the governed
modules are every file under `crates/core/src/build_adapters/` --
`mod.rs`, `registry.rs`, `matrix.rs`, `jvm_common.rs` and `bounded_io.rs`
included, no file exempt by name -- plus any workspace file holding an
`impl BuildAdapter for ..`; adapters, their types and their ids are read
from those impls. Re-review 3 (`review/REVIEW-STACK-3.md` section 1)
found 31 of 43 audit slips were a hand-written list that did not contain
the thing; these rules keep no such list except the four bounded
primitives, each of which is itself checked to name its cap.

`docs/build-artifacts.md`'s support rows (first cell a backticked id,
second `implemented`/`planned`), the matrix's `Status::Implemented`
struct literals and the derived adapter ids are the same implemented
set in all three directions; every docs row's Actions cell is exactly
`inspection only`; every matrix entry's `actions` is `INSPECTION_ONLY`,
which must still be defined as `"inspection only"`. The executable
column-by-column half is
`build_adapter_contract::docs_table_equals_the_capability_matrix`
(status, families, operation granularity and actions per row, and the
set of rows).

**Limits.** The known-layouts and attribution-limits columns are prose
and are compared by review, not by equality.

## Runtime tests that complete it

- `crates/core/tests/build_matrix_docs.rs::docs_table_equals_the_capability_matrix`
