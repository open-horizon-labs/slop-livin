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

No adapter module may reference `fs::read_dir`, `read_dir(`, `walkdir`
or `jwalk`; resolved calls catch `use std::fs::read_dir as list`, and a
discarded result (`let _ = read_dir(..)`) is the same enumeration.

**Limits.** A syscall-shape check, not proof that folded rows are reused.
The work counters (`dirs_listed`, `files_statted`) prove the reuse, and
the unchanged-container cost test asserts they stay at zero.

## Runtime tests that complete it

- `crates/core/tests/build_adapter_cost.rs::unchanged_container_lists_nothing`
