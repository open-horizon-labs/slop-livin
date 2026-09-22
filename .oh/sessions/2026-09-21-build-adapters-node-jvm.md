---
date: 2026-09-21
outcome: disk-growth-by-project
issues: [64, 65, 67, 68]
---

# Build adapters: the trait, the matrix, Node and the JVM

## The audits landed first, failing (GUARDRAILS_SPEC.md section 18)

Eight audits registered in `crates/source-audit/src/build_audits.rs`
before any adapter existed, so the baseline is the audit's own
enumeration rather than a prose list. The repo-level run
(`cargo run -p swamp-source-audit`) at commit 1:

```
FAIL  build_adapters_are_pluggable: crates/core/src/build_adapters/ has no adapter modules
FAIL  build_adapters_are_inspection_only: (same)
FAIL  build_adapters_read_bounded_manifests_only: (same)
FAIL  build_adapters_do_not_traverse: (same)
FAIL  build_units_built_through_builder: (same)
FAIL  build_adapter_test_contract: (same)
FAIL  build_adapter_matrix_matches_docs: (same)
FAIL  build_adapters_reuse_under_event_coverage: read crates/core/src/build_adapters/mod.rs:
      No such file or directory
8 audit(s) failed
```

Every one of the eight carries three rejection fixtures in the mutation
corpus, including an alias/rename variant and, where the audit is about a
result rather than a call, a discarded-result variant --
`NOT_YET_IN_THE_CORPUS` stays empty.

A deliberate choice in the audit shape: `adapters_or_err` makes an
*absent* `build_adapters/` directory the first failure of seven of the
eight rules, rather than letting them pass vacuously over an empty
directory. An audit that passes because the thing it governs does not
exist is the failure mode section 18 exists to prevent.

## The port, and what it changed

Cargo's identification moved out of `cargo_artifacts.rs` into
`build_adapters/cargo.rs` with its unit ids, its `cargo-folded-v2`
evidence source and its role assignments intact, so a stored report
written before the port replays after it. What changed is the shape, and
each change is one of the section 18 rules: the recursive `fs::read_dir`
descent became folded walk rows plus three capped `shallow_list` calls;
`fs::read_to_string` on fingerprint JSON became
`bounded_io::read_manifest`; the units are built through
`NestedUnitBuilder`. `cargo_artifacts.rs` keeps only what is not
adapter identification -- the direct `inspect_target` API the TUI
fixtures and cleanup rechecks use, plus layout resolution -- and its
`.cargo/config` read is now bounded too, because a guardrail satisfied
only by where the code lives is not satisfied.

## The reuse gate got stricter, and a test had to say so

The old Cargo consumer decided reuse from the FSEvents replay's
`changed_paths` list being empty. That list says "this replay reported
nothing"; it does not say "nothing changed since your rows were
written", and the difference is a recorded window start. A forced full
walk deliberately leaves the stored FSEvents anchor alone, so the pass
after one has an empty change list and **no** window start -- and the
old code reused anyway.

`build_adapters::BuildCtx::container` gates on
`EventCoverage::unchanged_since` and nothing else, which refuses that
case. `cargo_delivery::trusted_unchanged_container_reuses_units_without_reading_fingerprints`
had to gain two warm-up observations to establish a real window (the
store only records `last_observed_at` on its second pass), and a new
sibling test pins the refusal:
`without_a_recorded_window_start_the_container_is_identified_again`.

This is a real behaviour change, not a test fix: a report that could
previously replay stale rows indefinitely now re-identifies until a
window exists.

## Measured cost

`crates/core/tests/build_adapter_cost.rs::cost_report_cold_unchanged_and_one_container_changed`,
one machine, debug build, fixture = a 300-package `node_modules` plus a
`dist/`:

```
fixture: 300-package node_modules + dist, 302 identified units
cold (no cache, no window):  10.76ms  dirs_listed=0 files_statted=0 manifest_bytes=7490 reused=0 identified=2
unchanged (trusted window):   1.12ms  dirs_listed=0 files_statted=0 manifest_bytes=0    reused=2 identified=0
one container changed:       10.44ms  dirs_listed=0 files_statted=0 manifest_bytes=7490 reused=1 identified=1
```

Three things to read out of that:

- **An unchanged pass reads nothing.** Zero listings, zero stats, zero
  manifest bytes -- the number the docs claim, asserted rather than
  described.
- **`dirs_listed` is zero even on the cold pass**, because the structure
  came from the folded walk's own rows. The listings the adapters do
  make are for named leaf directories (a Cargo profile, a Maven
  `target/`), which this fixture does not exercise.
- **A one-container change costs that container, not the report.** The
  `dist/` container was still replayed while `node_modules` was
  re-identified. The absolute numbers are close here only because the
  fixture has two containers and one of them holds all 300 manifests.

The 300 packages cost 7,490 manifest bytes, not 300 whole
`package.json` files: the budget is 200 manifests, largest first, and
the other 100 packages carry an explicit "sized but not identified"
limit naming the number
(`a_manifest_budget_bounds_a_large_installed_tree`).

## What is written, tested and not yet wired

The adapters identify shared stores -- an npm `_cacache`, a pnpm object
store, a Gradle user home's `caches`/`wrapper/dists`, a Maven local
repository -- and those paths have tests
(`build_adapter_cost::a_shared_store_container_is_replayed_on_the_same_gate`,
and the store tests in each adapter). `consumers/cargo.rs::shared_containers`
returns an empty list.

That is deliberate and stated in the code. These are detector-resolved
external locations whose measurement and history window
`external::discover_and_measure` already owns; joining them in from the
build consumer would be a second observation of the same bytes in the
same pass, which is the shape `history-sweeps-are-owned` exists to stop,
and deciding the ownership is #47/#57's seam rather than this chunk's.
Wiring a second traversal to make a column non-empty would have been the
wrong trade.
