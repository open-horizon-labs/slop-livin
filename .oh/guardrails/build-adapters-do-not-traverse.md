---
id: build-adapters-do-not-traverse
severity: hard
statement: "Build adapters do not walk the filesystem. Directory structure reaches them through the folded walk rows in their identification context, or through the capped `locations::shallow_list` for the single-level listings a layout genuinely requires."
outcome: disk-growth-by-project
audit: build_adapters_do_not_traverse
---

## Rationale

The adapter-scoped half of `no-second-traversal-on-report-path.md`, aimed
at the two largest directory trees on a typical developer machine:
`node_modules` and `~/.m2/repository`. The folded walk has already
measured both. An adapter that walks either turns an ordinary refresh
into a second full scan of exactly the bytes that were just counted --
and, unlike the agent homes, these trees are big enough that the
regression would be measured in minutes.

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

Every governed function fails if it reaches `fs::read_dir` or anything
in the namespaces `walkdir`, `WalkDir`, `jwalk`, `glob`, `walk`,
`attribution`, `folded_measurement` -- including a `BuildCtx` method in
`mod.rs`, a helper in another module (`ecosystem::detect` lists a
directory), `vec![std::fs::read_dir(p)]`, and `let list =
std::fs::read_dir; list(p)`.

**Limits.** The call graph is lexical: trait-object dispatch
(`adapter.identify(..)` through `dyn BuildAdapter`), function pointers
stored in a struct and closures passed across modules are not followed,
and a method call resolves only to the `impl`s of types the calling
function names in its signature or body. A primitive *named* anywhere in
a governed function (a function pointer, a type) is flagged even when it
is not called. Unknown macros in a governed module fail the rule.

## Runtime tests that complete it

- `crates/core/tests/build_adapter_cost.rs::unchanged_container_lists_nothing`
