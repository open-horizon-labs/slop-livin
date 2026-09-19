# Rust artifact review corrections and selective cleanup

> Superseded completion assessment: the 2026-09-19 dissent found the eager model,
> walk and persistence violate the incremental-cost requirement. See
> [salvage and replacement solution space](2026-09-19-nested-index-restart.md).
> Passing tests below do not establish architectural readiness to merge.

## Aim and scope

Follow-up to #66 under #74: make the worker's nested Cargo facts trustworthy,
then enable exact, explicitly reviewed cleanup in CLI and TUI. Identification
comes before cleanup. This does not complete the cross-ecosystem epic.
No real build artifacts were removed; cleanup ran only against disposable fixtures.

## Delivered

- Canonical-container-scoped IDs, independent of role and inferred owner.
- Correct nested growth keys and root totals without parent/child double counting.
- FSEvents path invalidation, including same-size changes and removed roots;
  failed reads do not overwrite history with apparent deletion.
- Explicit target-triple layouts and ordinary Cargo fingerprint test detection.
- Linear history annotation; shared draft data and compressed report cache.
- Exact test/example executable groups with dep-info/debug-symbol companions,
  and individual incremental/build-script directories. Shared dependency groups
  and unsupported layouts remain inspection-only.
- CLI explicit plans and TUI Builds selections use the same locked revalidation,
  authorization, ledger, and same-filesystem Trash envelope with restore manifest.

## Verification and risk retirement

| Risk | Evidence |
|---|---|
| Identical relative paths collide across roots | `review_cargo_regressions` independent-root fixture |
| Growing files lose history | 4096-to-8192 regression; total history excludes nested double counting |
| Equal totals hide renames/removals | Incremental versus full facts and event-pipeline regression |
| Failed reads appear as deletions | Incomplete traversal regression and consumer fail-closed boundary |
| Cleanup removes a newer neighbor | Exact executable and directory-group fixtures preserve neighbors |
| Stale review silently expands | Same-size content, added companion, added directory member tests |
| Unsafe sharing or activity | Hardlink, symlink, missing-lock and held-lock refusals |
| TUI bypasses explicit authorization | TUI confirmation fixture rejects index-origin grant, then executes reviewed core plan once |
| Parent/child marks overlap | Core overlap fixture and pre-execution TUI rejection |

Workspace tests and source audit pass. TUI Builds row test verifies selectable
incremental entries and inspection-only dependency groups. Live read-only scan
of swamp identified 256,504 nested entries, including 154 test executables.

## Performance evidence and limitations

Debug-build benchmark (`cargo_inspect_bench`) on the real swamp directory:
about 23 seconds full, 25–28 seconds unchanged incremental. Report/store bytes
dropped from approximately 381 MB to 41 MB after compression. These are debug
measurements, not a production performance claim. Trace shows report loading,
history processing and cache serialization dominate; zero-change filesystem
replay itself takes under a second. Further report-cache/history optimization
is needed before calling large nested refreshes fast.

Only already observed build roots are modeled. Cargo configuration discovery
is not a complete implementation of Cargo's merged ancestor/global configuration
or all path templates. No last-execution or obsolete-generation claim is made.
Fingerprint evidence is identification evidence, not a deletion verdict.

Cleanup requires supported local filesystems, established Cargo profile locks,
and a successful bounded occupancy probe. Advisory locks cannot protect against
manual writers that ignore Cargo locks; users must stop those writers. Linux
runtime behavior was not verified on this macOS host. Human review should check
the destructive-action boundary before merging. No release or real cleanup was
performed as part of this correction.

## Review decision

Aligned with identification-first selective cleanup. Correctness regressions
have adversarial checks. Delivery is locally verified, with the performance,
configuration coverage, and cross-platform limits above explicitly retained.
Do not close #74 or claim full generic cleanup coverage from this Rust work.
