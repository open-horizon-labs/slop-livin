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

## Second session (2026-09-22): the takeover

Picked up from a stopped worker with five commits and an uncommitted
audit rewrite. What this session decided, measured and found.

### Aggregation now counts what it claims to

`summarize_container` replaced the family sum. The old summary counted
every unit an adapter emitted, so an unsupported layout inflated a
family and an empty directory was a "candidate". Now: nonempty supported
candidates only; unsupported units are one "Not identified" figure;
**whatever the outermost units do not account for is reported as
unaccounted bytes** (a container's loose files, a Maven artifact
directory's metadata files); members adding up to more than their
container is `None` ("not reconciled"), never a residual of zero, because
it can only mean a double count upstream. Each family carries its
largest member's consequence plus how many other consequences it hides,
and guidance from `family_guidance` (<= 32 characters, asserted, because
that is what the 80-column advice column shows whole).

### Maven origin: the file's presence was the wrong evidence

The first cut treated `_remote.repositories` existing as "downloaded".
Maven Resolver's enhanced local repository manager writes that file for
`mvn install` too, with an empty repository id (`LOCAL_REPO_ID = ""`,
checked against the Resolver source), so presence proves nothing. The
entries are now read (bounded; the file is a few hundred bytes) and a
`*.lastUpdated` file -- a record of a resolution *attempt* -- no longer
counts as a download on its own. Tests:
`an_empty_repository_id_is_a_local_install_not_a_download`,
`a_remote_repository_entry_means_downloaded_from_that_repository`,
`a_last_updated_marker_alone_leaves_the_origin_unknown`,
`a_malformed_origin_file_is_an_explicit_unknown`,
`artifact_level_install_metadata_marks_only_the_versions_it_lists`,
`a_local_only_artifact_in_a_project_output_and_repository_stays_distinct`.
An unresolved `${property}` version directory is now a residual *unit*
(the previous test asserted it disappeared while its comment said it
must not).

### Two real bugs the real-pipeline tests found

`crates/core/tests/build_adapter_history.rs` drives
`report_full_mode_with_source` with a scripted event source. Two of its
first five tests failed:

1. **A marker change was replayed away.** Which adapter claims a
   directory is decided by marker files *beside* it; adding
   `settings.gradle` next to a Node project's `build/` produces no event
   under `build/`, so the container gate replayed the Node adapter's
   rows indefinitely. Replay now also requires the stored units to have
   been written by the adapter claiming the container this pass.
2. **An unreadable directory inside an artifact vanished.** The walk's
   size job swallowed the `read_dir` error and every ancestor row said
   `complete: true`: a `node_modules` with one `chmod 000` package read
   as a complete tree 24 KiB smaller, with growth `-24576`. The walk now
   records the unlistable directory as an incomplete zero-byte row and
   `aggregate_dir_totals` rolls completeness up with bytes. This is a
   walk change outside `build_adapters/`; the full core suite passes
   with it.

### Presentation by role, not by id

The TUI had three `adapter != "cargo"` comparisons deciding
presentation. They are gone: a container whose units carry roles
`cargo_cleanup::speaks_for` keeps Cargo's purpose groups; any other
identified container gets neutral family groups (closed until opened;
members oldest first, unknown last, 25 shown then "... and N more"; a
Not identified row; every row `blocked`, so Space is refused as
inspection-only). The pluggable audit now rejects that shape anywhere
in the workspace (fixture `06-id-dispatch-in-the-tui`).

### The audits, in derived-set form

`crates/source-audit/src/build_audits.rs` was rewritten against
`review/REVIEW-STACK-3.md` section 1's five structural causes:

| Cause | Before | Now |
|---|---|---|
| Hand-written file lists | `mod.rs`, `registry.rs`, `matrix.rs` exempt; adapters = files under one directory | every file under `build_adapters/` governed; plus any workspace file with `impl BuildAdapter`; adapters, types and ids read from the impls |
| Hand-written name lists | `actions::propose` etc. by name | namespaces (`actions`, `grants`, `ledger`, `execution`, `cargo_cleanup`, `Command`, `process`, `OpenOptions`, `trash`, `walk`, `walkdir`, `jwalk`, `glob`, `attribution`, `folded_measurement`); the one list kept (`BOUNDED_PRIMITIVES`) is checked to still name its cap |
| Name checked, not behaviour | registry "contains" the adapter; test name present | registry count exactly one per derived `<module>::<Type>`; tests must be real `#[test]` fns in `#[cfg(test)]`, not ignored, asserting; gate call must not be discarded |
| Only the named function read | per-file primitive scan | whole-workspace call graph: free calls, calls in macro arguments, and methods resolved to the impls of types the caller names; primitives *named* (fn pointers) count too |
| Syntax the layer cannot see | `vec![read_dir(..)]` invisible | macro arguments parsed as expressions; unknown macros fail |

Builder bypass by later mutation (re-review 3's `u.protected = false`)
is caught in every form I could write: assignment, `+=`, `&mut` borrow
(`mem::replace` under an alias), `ref mut`, any method outside a
fail-closed read-only allow-list, inside macro arguments, and one call
away (a function elsewhere that takes a unit mutably and writes a field,
or builds a `NestedArtifact` literal, taints its governed callers --
`cargo_artifacts::inspect_target` is one). The field set is parsed from
`artifact.rs`. The sanctioned enrichment, `NestedUnitBuilder::amend`,
is an `expect: accept` fixture.

Corpus: 54 build-adapter fixtures across the eight audits (each >= 5,
each with an alias/rename and a discarded-result variant), including
helper-one-call-away variants for all three primitive rules. See the
verification section below for the run.

**Limits stated in the guardrails:** trait-object dispatch, function
pointers stored in structs, and methods on types the caller never names
are not followed; the field match is by name (a false positive is
possible, a miss by renaming a field is not).

### Real-pipeline cost

`build_adapter_history::cost_report_real_pipeline_unchanged_and_one_group_change`,
debug build, this machine, 301-package `node_modules` + `dist` +
`coverage`, one walked root:

```
cold                         144.42ms dirs_listed=316 files_statted=942 manifest_bytes=7489 containers_reused=0 containers_identified=3
unchanged                     63.91ms dirs_listed=0 files_statted=0 manifest_bytes=0 containers_reused=3 containers_identified=0
one group changed (dist)      99.25ms dirs_listed=1 files_statted=2 manifest_bytes=0 containers_reused=2 containers_identified=1
store bytes: before=39018 after_unchanged=39028 after_one_group=46773
```

The baseline for "vs baseline" is the pipeline's own walk: the one
listing and two stats on the changed pass are the incremental walk
re-measuring `dist/`; the adapter layer adds zero listings, zero stats
and zero manifest bytes to both the unchanged and the one-group pass. An
unchanged pass grows the store by 10 bytes (the checkpoint), a one-group
change by 7.7 KB (the reverse delta for the changed rows). I did not
re-run stack/13 in a second worktree for a side-by-side number (other
worktrees are off limits); `reviewer_cost_measurement_stack2` remains
red at exactly its stack/13 numbers (71 listings / 36,281 stats), which
is the evidence this chunk did not regress the unchanged pass.

### #65 adversarial matrix, mapped to named checks

| Case | Check |
|---|---|
| overlapping scopes | `identify_all` claims each path once (`every_ecosystem_in_one_checkout_is_identified_by_its_own_adapter`, `an_ambiguous_directory_goes_to_the_ecosystem_whose_marker_is_present`) |
| partial/unreadable containers | `an_unreadable_sub_directory_is_incomplete_coverage_not_a_disappearance`; per-adapter `an_incompletely_measured_*` |
| changed classification, identical bytes | `reclassification_and_new_nesting_never_reach_history_as_growth` |
| parent plus child aggregation | `a_family_summary_stops_at_the_outermost_unit`, `family_summaries_never_exceed_their_container`, `leaf_totals_reconcile_to_the_container_through_an_explicit_residual` |
| shared cache entries | `a_pnpm_virtual_store_is_not_charged_twice`, `a_shared_store_container_is_replayed_on_the_same_gate` |
| replacement at the same path | `node_full_and_incremental_agree_as_a_sub_artifact_changes_vanishes_and_reappears` |
| full vs incremental agree | same test, three changes |

### Still open

- **Machine-wide stores are not joined into the live report.** npm
  `_cacache`, the pnpm store, a Gradle user home and `~/.m2/repository`
  are identified by the adapters (unit-tested) but `shared_containers`
  returns nothing, for the ownership reason above. #67's "repository
  artifact coordinates" and #68's "npm/pnpm storage" are therefore met
  in the adapters and not in a user's report. The next step is to have
  `external::discover_and_measure` hand the folded rows it already
  produces for those detectors to the matching adapter (a detector
  capability, not a detector id), under the External key family's
  ownership, with container reuse keyed the same way.
- Workspace-hoisting attribution and per-crate dependency sizing (#107)
  remain named limits.
