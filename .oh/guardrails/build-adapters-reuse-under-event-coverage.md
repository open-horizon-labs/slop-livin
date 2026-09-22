---
id: build-adapters-reuse-under-event-coverage
severity: hard
statement: "An unchanged build container is replayed through the same `EventCoverage`-gated container seam the agent adapters use. Never through a directory modification stamp alone."
outcome: disk-growth-by-project
audit: build_adapters_reuse_under_event_coverage
---

## Rationale

stack/13 added the coverage gate because a directory's own stamp does not
move when a file inside one of its subdirectories changes. "The stamp is
the same" is therefore not "nothing changed", and a reuse decision built
on it is a report that stops updating and never says so.

The build side is where that matters most. A `target/` or a
`node_modules` accumulates changes deep inside while its top-level stamp
sits still for weeks, so stamp-only reuse would replay a stale
identification indefinitely -- and the user would see a confident,
wrong, aging answer rather than an honest "not observed".

## Detection

`build_adapters/mod.rs` must consult `EventCoverage`. Any function in it
that decides reuse or replay from `mtime`/`mod_time` must have a coverage
term in the same function, so the gate is part of the decision rather
than present elsewhere in the file.

**Limits.** Function-scoped: the audit proves the coverage term is in the
deciding function, not that it is the controlling condition. The runtime
complement is the pair of cost tests below, which assert zero listings on
an unchanged container *and* a re-identification when coverage is absent.

## Runtime tests that complete it

- `crates/core/tests/build_adapter_cost.rs::unchanged_container_lists_nothing`
- `crates/core/tests/build_adapter_cost.rs::without_trusted_coverage_the_container_is_identified_again`
