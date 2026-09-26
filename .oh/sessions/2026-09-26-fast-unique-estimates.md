# Fast refresh, explicit unique-byte reconciliation

## Execute — selected policy

The owner selected fast refresh with explicitly stale unique-byte estimates
until reconciliation (#131). Do not add an inode inventory, database, or
automatic same-device rescan. Keep existing folded rows and typed Parquet.

Success: changed-container refresh stays local; current allocation and last
reconciled unique bytes are distinguishable in live and cached output;
explicit full observation measures shared inodes once across the measured
scope. Charge reassignment must not become claimed storage growth.

Pre-flight: scope is measurement/evidence, not cleanup authorization, install,
release, or perfect auditing. Existing report-only CLI and TUI cleanup remain.
The trade-off is stale unique-byte estimates between reconciliations, explicitly
approved by the owner. Partial coverage must not be labeled a complete scope.

Risk checklist (before editing):
- Cross-root links: fixture must reject summing root-local dedup totals.
- Device keys/root order: reject inode-only keys and first-root charge policy.
- Fast refresh: counters must reject rescanning unchanged sibling containers.
- Persistence: cached report must retain stale state and reconciliation age.
- History: no growth derived from switching unique-byte accounting bases.
- Coverage: partial/reused measurements cannot clear stale evidence.
- Storage: no per-file identity persisted, no new storage engine.

Dissent: an ephemeral ledger alone cannot reconcile reused roots. Therefore
only an explicit complete full measurement can refresh the scope-wide unique
estimate; ordinary refresh retains its last value as stale. Row allocations
remain useful for cleanup, and are not promised reclaimable bytes. Stop if
correctness requires silently broadening normal refresh into a global walk.

## Implementation / trade-off

An explicit `observe --full` uses an additional pass through the existing
parallel folded walker with one device/inode set across measured roots and
units. This avoids coupling every adapter identification path to accounting
or persisting file membership. It increases explicit-full cost; normal refresh
does no census. The scope accounting overlay does not rewrite root-local
charges or history. Per-row shared-with attribution is still separate work;
do not close all of #131 on this implementation.

The existing run table stores nullable unique bytes, reconciliation timestamp
and needs-reconciliation flag. Invalidation precedes observation so partial
family updates cannot leave the prior value certified current. Cold reports
restore the same state. Unknown is not zero. Incomplete full coverage cannot
clear the flag. The source audit reserves “stale” for cleanup verdicts, so the
delivered field is `needs_reconciliation`, describing measurement evidence.

Unowned measurement boundaries distinguish direct/folded shared rows and
unreconciled estimates. Hardlinks no longer force unrelated-root traversal;
unknown old boundaries and ownership transitions retain full fallback.

Initial evidence: all 16 build-adapter/history tests pass, including shared
unowned refresh with at most one directory listed. Three reconciliation tests
pass: device keys, excluded files/symlinks/missing paths, 20,000 shared entries.
The 20k fixture has 1,000 distinct inodes and 1,792 set slots: 28,672 bytes of
key capacity (not whole-process RSS; excludes allocator/hash-control overhead).
There are no retained identities or new store files.

## Review / dissent

Independent Luna review found no concrete blocker after six focused scope
tests passed. Parent review strengthened the external fixture: a nested cache
alone would also pass if external roots were accidentally omitted, so a disjoint
Cargo-home/project hardlink case now tests that actual failure mode. Scripted
events canonicalize paths and filter by requested root, matching the live
source rather than spuriously forcing every sibling root to rescan.

Known missing roots are not an incomplete read of an existing path. They stay
in coverage and contribute no paths to the observed-set estimate; this makes
unused default tool homes compatible with reconciliation. An unmounted volume
may also be missing, so neither the docs nor this aggregate claim to cover it.
Partial/inaccessible roots and incomplete units still prevent certification.
This changes no history/tombstone behavior.

Risk retirement:
- Cross-root/root-order/device/exclusion: scope fixtures plus walker key and
  pruning tests reject per-root sums, inode-only keys, and excluded traversal.
- Fast refresh: no-event scope fixture asserts zero listings/stats/header
  reads/spawns; changed-container fixture rejects traversing a 96-file sibling;
  shared-unowned fixture allows at most one listed directory.
- Cold persistence/partial updates: run-row round trips and partial-family
  invalidation tests reject a transient-only warning or old certified total.
- History: overlay writes only run facts; existing history regressions remain
  unchanged. No new delta or charge-assignment code exists.
- UI: all 30 frame tests pass; new narrow/wide-header assertions preserve the
  warning without changing unrelated snapshots. Unowned text also warns.
- Old optional columns: direct typed-Parquet regression rejects making the
  existing store unreadable merely because the estimate was never recorded.
- Limits accepted: full reconciliation costs an additional walk; ordinary
  estimates may be stale; no atomic audit, extent dedup or per-row shared-with
  mapping is claimed. Native macOS/Linux full validation remains a CI gate.

Decision: continue with the selected policy, not a new storage engine or an
automatic full rescan. #131 stays open for its broader row-attribution work.

## Final validation

Routine workspace gate passed at 14:12:47 local:
`/tmp/swamp-fast-unique-reviewed-check.log` (format, strict Clippy, all source
audits, release feature graph, workspace tests, named targets and greps).
Final focused checks also cover the last unowned-view warning: 16 build/history
tests and six scope-accounting tests, plus the 30 TUI frame tests. Four direct
unit checks cover the ephemeral ledger and absent optional run columns.
No user filesystem data was removed, and no binary was installed or released.
