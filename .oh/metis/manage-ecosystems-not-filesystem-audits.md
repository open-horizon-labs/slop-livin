---
id: manage-ecosystems-not-filesystem-audits
title: "Manage developer ecosystems, not exhaustive filesystem audits"
outcome: disk-growth-by-project
---

## What we learned

The user wants to understand storage growth and make project/build cleanup
decisions. I invented a stronger requirement: preserve every internal file's
identity and history, including files already deleted. That requirement was not
requested. It drove the wrong architecture through multiple optimizations.

The eager Cargo model produced roughly 381 MB of serialized state, reduced to
roughly 41 MB with compression. Those earlier figures describe temporary-store
measurements, not precisely isolated cache sizes. A subsequent prototype encoded
256,504 entries in 4.52 MB. Smaller encoding did not fix the unnecessary inventory.
The SQLite detour and per-file Parquet replacement both optimized the wrong
contract. Passing audit-like tests was not evidence of product fitness.

The existing folded walker already measures directory interiors, keeps aggregate
history, and reuses measurements under trustworthy event coverage. Cargo needs
to interpret those measurements, not duplicate the walk or retain every compiler
output. Keep profiles, build/incremental groups, and evidenced test/example
executables. Inspect additional detail when requested. Before destructive actions,
freshly check the selected group and its evidence, not unrelated build trees.

Unknown subgroup hardlink attribution is acceptable when displayed honestly.
It must not be rendered as zero or turned into a reclaimable-space guarantee.
Directory mtimes alone do not prove unchanged contents; event-coverage checks and
the existing fallback remain necessary. This lesson does not weaken authorization,
stale-selection checks, or the replay checkpoint's publication ordering.

## Evidence from the correction

- Normal-report Cargo annotation now projects the existing directory measurements.
  The all-file recording seam and prototype index were removed.
- The 5,000-extra-files regression keeps the same nested report and history-series
  counts, verifies aggregate growth, and checks persisted Parquet for hidden
  internal-file inventory.
- Cleanup rechecks the selected members and recorded fingerprint evidence under
  Cargo locks; it no longer calls the full target inspector.
- Read-only debug benchmark on the main swamp checkout, isolated temporary store:
  894 Cargo units, including 154 test executables; about 226 KB total store;
  3.10 s first observation and 1.50/1.49 s subsequent observations. These runs
  fell back to full walks (no stored event ID / replay too soon), not event-only
  refreshes. Numbers are one local sample, not a performance guarantee.
- Final-code repeat: 1.80/1.89/1.79 s, 224,511–226,428 bytes total temporary
  store, same 894 units and 154 tests; again all full-walk fallbacks. Workspace
  verification passed 241 tests (two opt-in diagnostics ignored) and 19 source
  audits. Trusted no-change reuse is separately covered by a fixture with
  unreadable fingerprint metadata that must not be reopened.

## Use this next time

Ask which decisions require retained detail before designing its storage. Test
the amount and kind of retained state, not just encoding throughput. Adding
ordinary files inside a folded group should not add per-file report/history rows.
If a proposal requires an exhaustive index, first establish the user need that
folding and scoped inspection cannot meet. Do not silently promote precision into
a product requirement.
