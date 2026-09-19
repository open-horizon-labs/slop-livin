# Nested artifact index: salvage and replacement solution space

> Execution direction confirmed by user: apply and test the existing folded-folder
> walk and Parquet/reverse-delta approach FIRST. Alternatives require demonstrated
> failure of that implementation, not anticipated complexity. The uncommitted
> SQLite prototype and dependencies have been removed; no SQLite path is selected.

## Aim / problem statement

Explain project/build growth down to observed files and support precisely reviewed
cleanup without making routine refresh proportional to all nested artifacts.
Preserve the full BA0 / #74 scope and its identification-first ordering. This is
design work, not authorization to implement, merge, release, or clean user data.

Binding constraint: with trustworthy event coverage, adding unchanged descendants
must not add descendant reads, decoding, copying, history annotation, or writes to
a routine refresh. Initial inventory, coverage recovery, explicitly requested full
exports and actual large changes may cost proportionally to their scope.

## Salvage

Salvaged 2026-09-19: worker commit 92e60e4 and correction commit 5336ec5.
The correction's completion assessment is superseded. Tests established specific
correctness properties, not suitability of the model or walk. No code is deleted
or reset by this salvage.

### Evidence and learnings

1. `consumers/cargo.rs` loads the entire report before deciding nothing changed.
   `cargo_artifacts.rs::copy_tree` copies unchanged descendants recursively, then
   clears/recomputes their aggregates. Reusing facts is not skipping their cost.
2. `consumers/growth.rs` fabricates ArtifactRow shadow records for every nested
   entry. `growth.rs::observe_and_annotate` loads the volume-wide current table;
   a change rewrites that table. Keeping Parquet alone does not fix granularity.
3. `report.rs` clones and serializes the full nested report, even after a no-op.
   Compression shrinks bytes on disk but preserves all-entry decode/encode work.
4. The base walker already measures artifact interiors. A second Cargo traversal
   duplicates filesystem work; a restart must address both paths, not just caching.
5. The real fixture had 256,504 entries and 154 identified test executables.
   Prior debug measurements were roughly 23s full and 25–28s unchanged refresh.
   The benchmark measures total temporary store size, so the roughly 381MB/41MB
   before/after figures should not be treated as precise cache-only measurements.
   Tracing nevertheless directly identifies full report load/write overhead.
6. File-level history cannot be recovered by first inspecting children after a
   file has disappeared. Lazy presentation must not become lazy observation.

Frame shift: a nested report reused as state -> a compact observation index from
which reports, historical queries, and action proposals are independently derived.

### Reuse versus discard

Keep as evidence: collision, growth, equal-size rename, coverage failure,
hardlink, lock, stale-selection, companion, authorization and neighboring-artifact
fixtures. Port tests to new interfaces; their existence is not proof of a new design.

Review for reuse: role/fingerprint parsing, evidence vocabulary, separation of
ownership from identity, exact-action rechecks, ledger and recovery behavior.
Reassess hardlink lifecycle, directory replacement, incomplete configuration,
concurrency and native rebuild behavior before treating these fragments as sound.

Discard as architecture: eager Vec<NestedArtifact> as authoritative state,
recursive copying of unchanged subtrees, report JSON as incremental storage,
shadow ArtifactRow persistence, all-entry sparkline generation, Cargo's second
full filesystem traversal, and whole-container scanning for a selected cleanup.
JSON remains an explicit export format, not the incremental index.

### Guardrails / missing context / ownership

Existing W1b coverage and W2a evidence guardrails still apply; this does not create
parallel versions of them. Existing BA3 already required no-change and one-change
cost checks: the parent review failed to enforce that gate. No worker delegation
or skill invocation transfers ownership of architectural acceptance from the parent.

New local acceptance rule: count entries visited, metadata calls, partitions and
bytes read/written, rows decoded, adapter evidence reads and allocations; timing
alone cannot retire a scaling defect. Include cold-process and warm-process runs.
Do not call a fast path complete merely because full scanning was avoided.

## Solution Space

Decision criteria, in order: truthful history/accounting; change-proportional work;
bounded memory and storage amplification; precise actions; understandable recovery;
reuse of existing columnar/reverse-delta machinery where it meets these requirements.
No migration support is required. Scope cannot be reduced to aggregates or Rust-only
epic completion to make performance pass.

### Candidates considered

| Option | Frame and approach | Assessment / cost |
|---|---|---|
| A: binary eager report | Local optimization: change encoding, share allocations | Reject. Retains all-entry walk/copy/history work; faster constants do not meet the constraint. |
| B: aggregate-only observation, detail on demand | Reframe: index roots and inspect children only on expansion | Reject for this aim. Small fast state, but cannot explain previously observed/deleted files without separately capturing child history. |
| C: partitioned observation index with lazy projections | Redesign: one measurement pipeline, bounded columnar current/reverse-delta partitions, indexed summaries | Recommended. Reuses store semantics, not the monolithic layout. Requires partition routing, atomic generation publication and bounded compaction. |
| D: transactional keyed current/history store | Redesign: row-addressable facts and indexes, demand-driven queries | Viable alternative if C cannot bound amplification simply. Requires a different persistence engine and validation; do not maintain it alongside an authoritative duplicate Parquet history. |

A assumes the current report-first frame. B challenges the need to observe detail
but fails historical scope. C and D separate observation from presentation; their
real difference is partitioned batch updates versus keyed transactional updates.
No specific new database engine is selected without a focused evidence comparison.

### Recommended data model (C)

These are logical contracts, not a finalized public schema:

| Record | Purpose / fields |
|---|---|
| Container summary | Storage scope/key, coverage state, committed observation generation, event checkpoint reference, aggregate logical/allocated charges, adapter/config versions, detail-index reference |
| Directory/entry facts | Parent key and byte-preserving basename, entry type, observed device/inode and lifecycle evidence, logical/allocated size, change metadata, presence and observation generation |
| Partition directory | Key/path ranges -> bounded current/delta segments; persisted lookup, not a manifest listing every file loaded on every refresh |
| Accounting relations | Indexed physical-file identity -> known path memberships and accounting owner; multiple consumers never duplicate physical charge |
| Domain facts | Role/variant codes and optional evidence references; deduplicated provenance/limits, with dependency links to metadata/config inputs |
| Historical changes | Changed prior facts, presence transitions and coverage, linked to committed observation generations; ownership/role changes distinct from byte growth |
| Presentation / action | Query results and fresh reviewed groups, not persisted copies of every rich report object |

Separate path identity, observed physical identity, and inferred build identity.
Do not equate device/inode with eternal identity across deletion/reuse, or rename
with a proven compiler generation. Unknown evidence stays unknown. Fingerprints
are cached change/evidence metadata, not proof of last use or safe deletion.

Partition within containers: neither one file per artifact nor one huge partition
per target. Bound rows/bytes per shard, including enormous flat deps directories.
Small root/profile summaries must not embed all descendants. Keep committed base
segments plus bounded change overlays/reverse deltas; compact bounded shards, not
the whole volume. Publish data and replay checkpoint as one committed generation
with serialized writers and crash recovery. Retain unchanged segments by reference.

### Walk / update contract

1. Initial scan or justified reconciliation: the platform measurement pipeline
   collects facts once, in bounded batches. Cargo consumes those facts plus bounded
   reads of relevant existing metadata; it does not independently rewalk target.
2. Normal refresh: validate coverage/checkpoint and scope/config versions. Route
   events to affected entries/directories and metadata dependents using the index.
   Do not recompute recursive hashes to prove that nothing changed.
3. Read/update only implicated partitions, physical memberships and ancestor
   summaries. Apply byte deltas and reverse facts; retain unrelated partitions.
4. Unchanged state: read summaries/checkpoint only for this subsystem. Do not
   decode children, regenerate their history series, or rewrite detail segments.
5. Detail/history: query requested container/role/page/window on expansion.
   Full JSON export may intentionally enumerate all details. UI queries must not
   block rendering or silently fetch the entire container behind pagination.
6. Cleanup: resolve selected membership/evidence from indexed facts, then recheck
   the selected paths and relevant locks/companions against the live filesystem.
   Cached permission or occupancy is not authorization. No whole-container walk.

Expected work is summaries + affected entries/partitions + ancestor/accounting
fan-out, not simply number of containers. Coarse directory events may require
listing all immediate children; uncertain subtree events may require reconciliation.
Lost event coverage cannot promise cheap correct updates. Preserve explicit fallback
reasons and unknown coverage. Events do not promise history of every transient
create/delete between observations.

### Risk retirement plan

All evidence dispositions below are REQUIRED checks, not claimed passed results.

| Risk / assumption | Planned disposition | Adversarial evidence (tempting patch it must fail) | Stop/pivot |
|---|---|---|---|
| Work still scales with unchanged descendants | Retired by evidence, pending | 10k/100k/1m unchanged entries, cold and warm; zero descendant decoding/stat/rewrites after valid replay (binary eager cache) | Counts scale with descendants |
| One change rewrites a giant container/current file | Retired by evidence, pending | Deep and flat layouts; one change touches bounded shards plus ancestors, with recorded read/write amplification (per-target partition) | Work scales with unrelated files except documented event relisting |
| Lazy UI loses deleted-file history | Retired by evidence, pending | Observe growth, then deletion, never expand until afterward; explain old path and bytes (aggregate-only observation) | Detail depends on current filesystem existence |
| Duplicate walk hidden in adapter | Retired by evidence, pending | Instrument base walk and Cargo reads jointly; one initial measurement pass; no unchanged descendant reads (optimize only Cargo cache) | Adapter repeats measurement traversal |
| Coverage/fingerprint false negatives | Retired by evidence, pending | Same-size writes, nested changes, dropped events, restart, config-only changes and permission loss; compare full reconciliation (root-mtime shortcut) | False unchanged/deleted state |
| Hardlinks or inode reuse corrupt accounting | Retired by evidence, pending | Cross-root aliases, link add/remove, replaced files and root-order changes; compare full accounting (independent subtree sums) | Need global scans for ordinary local link changes or false growth |
| Partitions become an unbounded mini database | Retired by evidence, pending | Many small updates/compactions; shard/manifest/memory amplification bounded; crash injection before/after generation publish (append forever) | C requires opaque recovery or unbounded compaction; evaluate D |
| TUI/export/history still materialize everything | Retired by evidence, pending | Query first page and one historical window; count reads/allocations; full export is explicit (cosmetic pagination) | First page scans all details |
| Cleanup needs stale or global state | Retired by evidence, pending | Prior adversarial tests plus large-container single-group proposal and native rebuild of retained output (trust cached group or scan everything) | Broader action or observation than selected evidence requires |
| Useful performance only in one platform/build | Retired by evidence, pending | Release-mode real swamp measurements and Linux event-gap fixtures; report cold/warm and phase cost (debug caveat as excuse) | Correctness depends on macOS-only hidden assumptions |
| Future need and user usefulness | Accepted with rationale | Future use is unknowable; Muness reviews whether identification/history answers the question, no automatic obsolete verdict | UI implies unknown means safe |

### Dissent on the recommendation

Best defense of C: existing columnar/reverse-delta semantics can survive if the
physical layout and query boundaries change. Counterargument: shard routing,
atomic publication and compaction can recreate a database badly. Confidence in
separating facts/projections is high; confidence in C over D is medium.

Pre-mortem: (1) broken event or commit coverage produces false history;
(2) deferred detail freezes the UI on first expansion; (3) custom storage plumbing
consumes effort that a keyed engine could have saved. The checks above target each.
Weakest assumption is that C remains simpler while satisfying amplification and
recovery bounds. ADJUST: select the logical design, provisionally prefer C, and
require its storage/walk proof before UI/cleanup integration. Pivot to D rather
than patching around a failed proof. This gate is sequencing, not a reduced MVP.

### S&T selection / execution handoff

| Step | Disposition | Parent | Sufficiency | Owner | Review trigger |
|---|---|---|---|---|---|
| BA0 | selected, unchanged | W0 | GBA contributes to W0 | unassigned | Full capability scope reduced |
| BA1 | selected | BA0 | GBA | unassigned | Evidence or paged detail cannot explain builds |
| BA1b | selected, replacement mechanism C provisional | BA0 | GBA | unassigned | Scaling, false history, recovery failure |
| BA2 | selected, integration after index proof | BA0 | GBA | unassigned | Scope/recheck or retained-build failure |
| BA3 | selected, gating rather than advisory | BA0 | GBA | parent review; implementer unassigned | Any required check missing/failing |

GBA remains BA1 + BA1b + BA2 + BA3; all required. Existing #64–#76 lineage and
other ecosystems are preserved. No new GitHub issues or scope changes made here.
Rejected tactics: eager report cache, recursive unchanged copying, aggregate-only
history. Deferred alternative: D until C's falsifiable gate; not a second store.

Preserve history scope, honest evidence, precise authorized actions and existing
coverage guardrails. No migration layer. Accepted costs: initial inventory,
proportional real changes, explicit gap reconciliation, full requested exports,
bounded compaction and ecosystem-specific evidence maintenance. No subsecond claim
until measured; structural counters are non-negotiable even if release timing is fast.

Next action after implementation authorization: demonstrate index/measurement
contracts on the scaling and historical fixtures before wiring rich reports back
in. Commit a storage ADR only after the proof chooses C or D. Parent reviews model,
walk, history and measured cost together; no delegation is authorized here.

## Execute — existing folded measurement path, 2026-09-19

User authorized implementation and explicitly corrected the unjustified SQLite
detour. That prototype, its tests, module export, dependency and lockfile additions
are removed. Its running test processes were stopped. No production data removed.

Implemented the first measurement boundary in the existing parallel folded walk:
an optional bounded channel emits compact filesystem facts from `process_size`,
using metadata already collected for size accounting. Carried artifact roots emit
no interior facts. Files retain device/inode/link evidence before deduplication;
symlinks are represented without traversal, and read failures reach the consumer.
The existing zstd/Parquet writer now accepts bounded batches. Publication remains
replacement of a completed file, not a second database. Partial streams retain the
prior published file; temporary files have unique RAII-managed names.

`folded.rs` contains measurement columns, not rich NestedArtifact objects. Facts
are not yet the authoritative detail/history path. Do not call this full delivery.

### Evidence

- `folded_measurements`: same folded pass emits seven distinct fixture entries,
  retains hardlink identity, represents a symlink, and emits zero descendants
  when the artifact is carried. A separate columnar fixture preserves raw name
  bytes and tests bounded row groups and failed-publication retention.
- `folded_walk_bench` on the real swamp directory: debug initial measurement plus
  Parquet output 3.250s; 257,697 entries; 13,268,057 bytes. Carried pass 4.096ms,
  zero interior entries and zero detail bytes written. This is the measurement
  seam, NOT full FSEvents/report/history/GUI latency. No second Cargo scan runs
  in this probe. No real artifacts were modified.
- Workspace tests passed after the first integration seam. Final checks must be
  rerun after subsequent edits; no downstream end-to-end performance claim.

### Remaining required work / risk gate

Wire measurement segments into the existing scoped current/reverse-delta store;
replace eager Cargo annotation and shadow rows, route changed interiors through
the same measurement boundary, query history/details on demand, and retain exact
cleanup rechecks without a whole-container scan. Hardlinked changed-root behavior,
cross-root accounting, event checkpoint consistency, paged UI, one-change cost,
million-entry scaling, retained-build validation and Linux verification remain
required checks, NOT accepted omissions. The existing path has not failed; an
alternative engine is not justified. No merge, release, or cleanup authorized by
this measurement result. Scope and GBA sufficiency remain unchanged.
