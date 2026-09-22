---
id: agent-adapters-do-not-traverse
severity: hard
statement: "Adapters do not walk the filesystem. Directory structure reaches them through the folded walk rows in their identification context, or through the capped locations::shallow_list helper for the single-level listings a layout genuinely requires."
outcome: disk-growth-by-project
audit: agent_adapters_do_not_traverse
---

## Rationale

The adapter-scoped half of `no-second-traversal-on-report-path.md`. Each
of the fourteen adapters open-coded its own recursive `read_dir`, so a
report over a large tool home paid for the walk once in the folded walk
and again, per adapter, per call. Keeping this rule adapter-scoped as
well as path-scoped means a new adapter fails the build immediately
rather than quietly re-adding the cost.

## Detection

No adapter module may reference `fs::read_dir`, `read_dir(`, `walkdir`,
`jwalk` or `resize_artifact*`.

**Limits.** Same as the sibling guardrail: a syscall-shape check, not
proof that folded rows are reused. Work counters prove the reuse.

## Runtime tests that complete it

- `crates/core/tests/incremental_external_and_agent_measurement.rs`
