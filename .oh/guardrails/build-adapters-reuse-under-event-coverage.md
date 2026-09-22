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

Replay sites are derived: every governed function outside
`impl ContainerCache` that reads the cache's storage (`.cache.entries`)
or calls one of the cache type's non-constructor methods. Each must
reach, within its file, an `unchanged_since` call whose result is not
discarded. Separately, a governed function whose reachable surface
mentions reuse/replay and reads an `mtime`/`mod_time` with no honoured
coverage call fails. With no replay site at all the rule fails (reuse
is required, not optional). The runtime half: replay also requires the
stored units to have been written by the adapter claiming the container
this pass (`rows_written_by_another_adapter_are_never_replayed`), and
`build_adapter_history::an_unchanged_node_checkout_replays_its_units_without_reading_a_manifest`
measures zero manifest bytes on an unchanged pass.

**Limits.** "Honoured" is the resolver's `Honoured` classification: a
result bound to a variable that is later read counts as honoured even if
the read does not gate the replay. The runtime tests cover the gate's
behaviour.

## Runtime tests that complete it

- `crates/core/tests/build_adapter_cost.rs::unchanged_container_lists_nothing`
- `crates/core/tests/build_adapter_cost.rs::without_trusted_coverage_the_container_is_identified_again`
