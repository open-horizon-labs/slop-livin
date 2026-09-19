# Nested artifact index: salvage and replacement solution space

## Current direction — supersedes the design and binding criteria below

The user explicitly corrected the frame: this manages a developer ecosystem,
not perfect filesystem audits. Exhaustive internal-file/deleted-file history was
an assistant-invented requirement. The per-file index prototype is discarded,
not awaiting integration. All earlier plans and benchmarks below are historical
evidence, not instructions to resume that architecture.

Implemented direction: project compact Cargo groups from the existing folded
directory measurements; retain evidenced test/example executables; keep aggregate
history; reuse trusted unchanged observations. Ordinary compiler files stay
folded. Deep inspection is explicit, and cleanup freshly checks only its selected
group and evidence. The replay checkpoint publication fix is retained.

Learning: [manage ecosystems, not filesystem audits](../metis/manage-ecosystems-not-filesystem-audits.md).

### Incremental hardlink update — subsequent execution

User selected faster incremental artifact updates with explicit measurement limits.
The shared walker now relists affected directories even when an artifact contains
hardlinks. Path allocations stay current; unique-byte totals retain their last
measurement with a persisted `dedup_stale` flag until a full scan reconciles them.
Unique-byte history has a gap while stale. Allocated size/growth is surfaced on
artifact rows; CLI/TUI label stale totals. This is not a reclaimable-space estimate.

Wide directories use the existing walk worker pool with at most 256 temporary
entries per worker; no file inventory is persisted. Native directory-level events
still require relisting all siblings in an affected directory. On this checkout,
`target/debug/deps` alone held 186,493 entries. Removing the whole-target walk
does not make that directory free to measure.

Read-only benchmark, debug build, isolated store, injected notifications (not OS
delivery timing or actual source/build mutations), three samples per case:

- unchanged report: 189–195 ms;
- source directory notification: 207–213 ms;
- huge Cargo deps directory: 761–819 ms, down from 1.48–1.50 s;
- small Cargo incremental group: 278–283 ms.

The unchanged-report fixed overhead remains; this execution targets the dominant
hardlink fallback, not a wholesale cache/enrichment redesign.

Review: existing/new hardlinks use allocation rollups without double-charging
unique totals; stale status survives persistence and clears on full observation;
untouched unreadable subtrees retain measurements; history gaps and stale-baseline
growth are tested; parallel measurement skips symlinks and descendants. Cleanup
retains fresh selected-group checks. Stale estimates cannot spend standing-grant
budgets; explicit human plan approval remains possible with a warning. No live
user-store rewrite, real cleanup, push, merge, or release is authorized here.

Remaining human verification: whether the stale/allocation labels make the intended
decision clear in daily use. Synthetic events verify processing cost, not native
event-delivery latency. Full scans remain the reconciliation mechanism.

Final verification: 245 tests passed with `cargo test --workspace --quiet --
--test-threads=1` (two opt-in diagnostics ignored); all 19 source audits and
`git diff --check` passed. A parallel run had one lock-acquisition fixture failure
at proposal time; the test passed on its own and in the full serial suite. Cause
not established; no lock checks were relaxed. The existing raw zero local-charge
value is now preserved when reading Parquet, avoiding repeated no-op rewrites of
folded annotation rows whose unknown subgroup charge is represented by zero.

## Historical exploration (superseded)

### Release review follow-through — 2026-09-19

Aim: close Rust identification and cleanup verification gaps while preserving
folded storage, selected-group safety checks, and explicit accounting limits.

Delivered: normal reports retain final executables and library outputs using
shallow profile-root enumeration (including target-triple profiles). Dependencies
remain folded; final outputs are inspection-only. Updated the projection evidence
version so cached old projections cannot indefinitely hide the new output rows.
Tests check symlink/metadata exclusion, cleanup refusal for final outputs, and
unchanged top-level accounting.

A native, offline, temporary Cargo fixture now tests build → identify → approve
→ Trash → rebuild → execute. It touches no user build artifacts. It passed.

Parallel stress reproduced the prior lock failure. Cleanup now explicitly unlocks
at the end of its critical section instead of relying solely on descriptor close.
A deterministic duplicate-descriptor test demonstrates the lifetime problem and
checks continued exclusion while the guard is live. Transient fork inheritance
is the likely explanation for the parallel-only symptom; that exact fork was not
instrumented. After the change, 20 consecutive parallel Cargo delivery suites
passed. The full parallel workspace suite passed 248 tests (two diagnostics
ignored); all 19 source audits and diff whitespace checks passed.

Review: aligned; no per-file inventory or expanded cleanup authority. Continue
for this change, Adjust for release completion. Per-crate dependency attribution,
broader native Cargo variant/custom-root checks, real FSEvents smoke testing,
release-profile verification and publication remain open. Human verification of
the CLI/TUI accounting language remains required. This is not a shipped release.

The following execution/review is current; the older exploration resumes after it.

### Execute and review — folded ecosystem units

Aim: useful build identification, aggregate history, and selective cleanup without
retaining a per-file compiler inventory. Status: implemented and locally verified.
The checkpoint publication correction and small reverse-delta compaction remain.
Removed the assistant-owned all-file prototype, recording hooks, benchmark/tests,
and dead measurement-specific encoding specialization. Historical tracked versions
remain available in Git; prototype findings remain below, not production code.

Verification: `cargo test --workspace --quiet` passed 241 tests; two opt-in store
diagnostics ignored. `cargo run -p swamp-source-audit` passed all 19 audits.
`git diff --check` passed. The real checkout benchmark used an isolated temporary
store and changed no source/build files there: 894 units / 154 tests, 224,511–226,428
bytes total store, 1.80/1.89/1.79 seconds. All runs used full-walk fallbacks, not
replay-only timing.

Risk retirement:

- Hidden per-file inventory: the 5,000-file regression checks report counts,
  history-series counts, aggregate growth, cached units, and persisted Parquet.
  An eager model merely compressed or hidden behind a cache fails this check.
- Duplicate Cargo walk: normal consumer calls only folded projection; fingerprint
  and example enumeration are shallow. Trusted unchanged-container reuse is
  tested with unreadable fingerprint metadata, which must not be reopened.
- Unsafe simplification of cleanup: existing stale-member, companion, hardlink,
  lock, authorization, neighboring-file, and directory-group tests pass. New
  evidence-change test refuses a changed fingerprint; an unrelated unreadable
  subtree no longer blocks selected-group cleanup.
- Lost group recency: propagate already-measured descendant timestamps and test
  an old group directory containing newer internal files.
- False accounting precision: group allocation remains measured; unknown subgroup
  hardlink charge renders unknown. Exact reclaimed-space prediction is not promised.
- Replay publication ordering: existing failed-cache/checkpoint regression passes.

Frame review: aligned with the user's correction. Exhaustive deleted-file identity
history is deliberately not delivered. Native event timing varies by machine;
the fixture verifies reuse, while real measurements honestly report fallback.
No production cleanup was performed. Final UI usefulness remains for the user's
judgment; safety fixtures are not a claim of independent human review. No push,
merge, release, or user-store rewrite is part of this execution.

### Older exploration resumes here — superseded

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

### Execute / review — collection overhead comparison

Replaced per-entry synchronized sends with directory-local transfer buffers of
256 entries and bounded batch queues. Entries in a directory share the container
path allocation. Parquet batches use 16,384 rows instead of 1,024. No measurements
or precision were dropped; the format remains typed Parquet/zstd using the existing
writer. This is still the measurement seam, not completed report/history rewiring.

The benchmark now compares the same warm root across walk-only, collect-and-drain,
and collect-and-persist modes, rotating their order. It checks attributed total
bytes against the baseline and uses only disposable output. Real root:
`/Users/muness1/src/open-horizon-labs/swamp`; 257,717 measured entries.

Three-run wall-time medians (2026-09-19):

| Version / build | Walk | Collect | Persist | Persisted bytes |
|---|---:|---:|---:|---:|
| Per-entry baseline, debug | 1.228s | 1.594s | 1.698s | ~13.37MB |
| Batched, debug | 1.301s | 1.329s | 2.398s | ~10.12MB |
| Batched, release | 1.249s | 1.219s | 1.293s | ~10.15MB |

The earlier 3.25s single sample was not a comparable baseline. A run overlapping
release compilation was excluded. Release collection is within timing noise of
the walk; persistence overhead was ~44ms in this small warm-cache sample. No old
release comparison or cold-filesystem-cache test was run, so neither a before/after
release speedup nor cold-start performance is established. Larger row groups
reduce size but regress debug persistence; do not conceal that trade-off.

Risk checks: full buffers and tails preserve every path exactly once, shared
container pointers are verified, receiver disconnection does not hang the walk,
raw path bytes round-trip through Parquet, failed output after a flushed batch
preserves the old file and removes its temporary file. Existing carry test still
requires zero interior entries on an unchanged artifact. Workspace/source-audit
checks cover compatibility, not remaining end-to-end architectural requirements.

Review: aligned with reusing the existing walk/store; performance comparison is
now meaningful at this boundary. The outstanding integration gates above remain
required. Do not present this benchmark as a complete incremental refresh.

### Execute / review — storage overhead, 2026-09-19

Kept the existing Parquet/zstd current + reverse-delta model. The byte/presence
delta selection was already sparse by row; directory/file deltas can also record
metadata changes. This change does not discard those fields or coarsen precision.

Tested a global encoding change on temporary copies of the real store. It made
the existing Parquet files larger (1,544,230 -> 1,579,769 bytes), so it was rejected
for existing schemas. Preserve their established settings. The new raw entry
measurement schema uses sorted bounded batches, delta-encoded integers and path
prefixes, retaining every field and nanosecond timestamp. One debug measurement
fell from ~10.15MB to 7,762,671 bytes for 257,733 entries. This remains a full entry
measurement dump, not the final folded historical index or an acceptable substitute
for the pending report/history integration.

Release confirmation (three warm-cache, order-rotated rounds; compilation excluded):
257,733 entries, median plain walk 1,114ms, collection 1,169ms, persistence 1,195ms;
median output 7,762,356 bytes. Persistence adds 81ms against the same-run walk
median. This small sample establishes neither cold performance nor full-report
refresh performance; the persisted data is still the raw measurement dump.

General storage improvement: compact eight small delta files when their combined
size is <=128KiB, in addition to the existing >20-file trigger. Group rows by
identity/time to improve locality. On copied real data, retaining every row:

| Delta family | Before | After |
|---|---:|---:|
| 16777231 artifact deltas | 61,177 B | 4,412 B |
| 16777231 directory deltas | 109,415 B | 17,005 B |
| 16777235 artifact deltas (below threshold) | 5,727 B | unchanged |
| 16777235 directory deltas (below threshold) | 10,846 B | unchanged |

The first two families shrink 87.4% combined, by amortizing repeated schema,
footer and compression costs, not dropping observations. This is not an 87%
reduction of the whole store. Existing current snapshots and user data were not
rewritten. Real-store comparisons are ignored diagnostic tests requiring explicit
SWAMP_ENCODING_INPUT; all output goes into temporary directories.

Compaction previously deleted source files before writing their replacement.
All three families now publish the completed replacement first. Failure-injection
fixtures block publication and assert source bytes survive, then remove the fixture
blocker and verify every restored field exactly. This is not a complete transaction
or concurrent-writer fix: interruption during input retirement can leave duplicates.

Checks include UInt64/Int64 extreme-value round trips (no truncation), raw name
bytes, all prior growth/history and no-op tests, small-delta size reduction, exact
real-row equivalence, and publication failure for artifact/directory/file deltas.
No SQLite, new storage engine, migration operation, or deletion of real artifacts.

Remaining schema-level decisions: separate ephemeral cleanup rechecks from retained
facts; fold small interiors using the existing observation contract; avoid storing
container/parent identity repeatedly; preserve all promised deleted-file history.
Do not silently achieve a size target by weakening that contract. Existing global
history rewrite/read granularity and rich-report integration remain unresolved.

## Execute — partitioned facts and publication ordering, 2026-09-19

User explicitly requested the unfinished architectural replacement next, including
broader fixes. Scope remains full BA0/BA1b/BA2/BA3, not a narrowed first increment.
No delegation, alternative database, migration, release or real cleanup occurred.

Implemented the selected Parquet mechanism's storage/query boundary in
`folded/index.rs`, plus bounded external ordering in `folded/sort.rs`:

- Current and sparse reverse facts use byte-ordered range trees over the existing
  Parquet/zstd writer. Leaves cap both rows (4,096) and path/scalar payload (2MiB);
  routing fanout is 32. Container identity is not repeated per fact.
- No-change checkpoint publication reads/writes no descendant partitions. Local
  changes copy only affected partitions and routing ancestors. Retired current
  segments are removed after publication, not retained as full snapshots.
- Reverse facts are packed in their own bounded tree keyed by path/time/generation.
  A historical value is a range lookup, not a scan of every observation. Prior
  absence, deleted paths, raw names, inode/link evidence and nanoseconds survive.
- Paged queries cap returned bytes as well as rows. Retention advances a coverage
  floor and prunes one bounded page per call; historical queries before the floor
  refuse rather than returning invented values.
- One generated HEAD commits the two roots and caller checkpoint. Shared reader
  and exclusive writer locks prevent retirement under live readers. PENDING
  recovery rolls back an unpublished run or finishes committed retirement.
  Publication-uncertain handles refuse further writes until reopened.
- Complete reconciliation consumes sorted measurements and infers deletion only
  after successful end-of-stream. It is the explicit full/event-gap path, not the
  normal no-change path. The external sorter uses bounded runs and fan-in 16;
  it performs no additional filesystem measurement.

The storage boundary is exercised by a real folded-walker integration fixture:
hardlinks, link removal, symlinks, current and deleted history. APFS rejects invalid
UTF-8 filenames, so actual raw-name creation is Linux-only; columnar raw-byte
round trips run on all platforms. Linux execution itself remains pending.

### Broader pipeline correction now on the normal report path

Inspection exposed a pre-existing checkpoint hazard: `observe_tracked_with_source`
advanced replay before Cargo/history/report caching could fail, and cache errors
were discarded. The report now calls `stage_tracked_with_source`; its walk consumer
holds the checkpoint until a successful `ReportCached` event. Cache errors
propagate. Topology/unowned writes precede replay-anchor publication.

The existing low-level walk API still commits immediately for its callers; the
report uses the staged API. Source audit now follows and checks that delegation
and applies its existing replay-before-full-walk guard to the staged function.
This preserves, rather than bypasses, the guard. A cache-publication blocker proves
the failed report leaves the previous replay sidecar byte-for-byte unchanged;
removing the fixture blocker and retrying commits the new event id.

This is safe retry ordering, not a transaction across legacy volume datasets.
The new index's generation contract is scoped to that index; it does not yet make
all existing report stores atomic or fix concurrent legacy report writers.

### Evidence and boundaries

Real target probe (all output disposable), release/warm filesystem cache:
256,504 facts; globally sorted raw Parquet 4,304,388B; complete partitioned index
4,520,629B. Walk plus external sort 3.372s; initialize from ordered input 1.147s.
Reopen/no-change checkpoint 66.3ms, zero child rows/partitions decoded or written.
One synthetic changed fact: 120.5ms, one 4,096-row current leaf read, 4,097 rows
written across current/reverse leaves; 72,314B partition/routing reads and 75,036B
writes. Fifty-row page: 0.677ms, one leaf. These are single-run boundary timings,
not complete report latency. Counters exclude lock/HEAD/journal metadata. The real
filesystem was not changed; the synthetic mutation affected temporary index facts.

10k, 100k and explicitly run release 1m fixtures retain the same one-leaf update
bound; routing reads are 1, 1 and 2 respectively. Synthetic data is repetitive:
its compressed sizes are not representative. Fresh handles remove any in-memory
index state but do not create cold OS caches or independent processes.

Adversarial checks cover byte-bounded long-path pages/reconciliation/retention,
equal-size rename, same-size metadata changes, same-second generations, late input
failure, unsigned/signed extremes, raw names, deletion before first expansion,
split/delete/reinsert against a reference map, reader/writer exclusion, failure
after replacement segments are written, and recovery before/after HEAD publication.
One hundred tiny observations remain packed in <20KB including current/index
metadata rather than retaining 100 Parquet footers. Tests simulate interruption;
they do not establish every power-loss/filesystem behavior.

### Review — Adjust; integration remains incomplete

Aligned with C and the existing walker/store. The mechanism has not failed its
storage-boundary proof; there is no evidence justifying an engine substitution.
The necessary broader checkpoint correction is applied to the normal pipeline.

The partition index is NOT authoritative for normal reports yet. It is currently
called by fixtures and the benchmark. Eager Cargo annotation, rich report caching,
shadow history rows, TUI filtering and whole-container cleanup rechecks remain.
Do not claim end-to-end speedups or completed replacement from this commit.

Required next integration: route complete measurement scopes from full AND
incremental walks to the index; remove the second Cargo walk; replace report state
with summaries and indexed domain projections; wire paged UI/exports/history and
selected cleanup rechecks. Domain/config metadata invalidation, external roots,
cross-root physical accounting and inode replacement need their full gates.
Measure sparse-deletion leaf occupancy/compaction and real many-update storage.
Independent cold-process, Linux gap and retained-native-build checks remain pending.
No model-checkable end-to-end risk is marked accepted simply because it remains.

The parallel workspace suite also exposed a transient lock-busy failure on a
reopened index. A duplicated-descriptor fixture reproduces the relevant lifetime
hazard (as with a concurrent fork before exec): closing the owner's descriptor
alone can retain its flock. An explicit-unlock RAII guard now covers successful
views and constructor failures. Multiple independent shared readers still block
a writer until the last view drops. The workspace suite passed after this fix.

Verification: workspace tests and source audit passed. The million-entry release
probe was run explicitly (it is ignored in the routine suite). Strict clippy found
15 pre-existing warnings in actions/artifact/Cargo/walk code; the new-code warning
was corrected, and ordinary clippy completed with no warnings in the added modules.
`docs/folded-index.md` records the implemented boundary, measurements and remaining
integration gates separately from shipped architecture claims.
