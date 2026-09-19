# Documentation accuracy report

Reviewed 2026-09-19 against the source tree at `fa66177`, the documentation changes, and the artifact-history fix prepared during this review. Product and usage claims below refer to current source; published v0.4.0 binaries predate the rename.

The editorial sequence was structure, factual verification, then plain-language editing. The reader is a developer deciding whether to use swamp or understand its implementation. The rewrite preserves concrete commands, the project/worktree/artifact model, history, and recovery distinctions. It removes unsupported exclusivity, generic speed promises, and duplicated reference material.

## Claim checks

Quoted legacy wording identifies claims corrected or removed. Other rows state the checked replacement claim.

| Claim | Status | Evidence | Correction or disposition |
|---|---|---|---|
| “The only tool that combines…” / no other tool has history | Unsupported | [StorageRadar's official site](https://storageradar.app/) documents snapshot history and comparisons. No exhaustive survey supports exclusivity. | Describe swamp's combination of capabilities; remove the competitive matrix and uniqueness claim. |
| The README's renamed v0.4.0 download URLs exist | Incorrect | [Release assets](https://github.com/open-horizon-labs/swamp/releases/tag/v0.4.0), checked through the GitHub API | Assets still use `slop-livin-0.4.0-*`. The install instructions now target v0.5.0, built from the renamed source; old names belong in migration notes. |
| Rust 1.92 is the minimum supported version | Needs citation | [Workspace manifest](../Cargo.toml) and [release workflow](../.github/workflows/release.yml) do not establish a tested minimum. | Say recent stable Rust; do not invent a minimum. |
| “Opens in milliseconds” | Unsupported | [TUI startup](../crates/tui/src/lib.rs) loads a cache when available but blocks for the initial uncached report. No general timing study was supplied. | Describe cached startup and background observation. |
| “There are no per-file rows” | Incorrect | [Growth configuration and file storage](../crates/core/src/growth.rs), [walker](../crates/core/src/walk.rs) | Directory rollups plus selected large-file rows; default threshold 1 MiB. |
| Folded artifacts are “never descended” | Incorrect | [Artifact sizing](../crates/core/src/walk.rs), [incremental interior updates](../crates/core/src/growth.rs) | Folding groups report rows; measuring an artifact can traverse it, and storage retains interior directory detail. |
| Incremental updates always remeasure only a changed directory | Misleading/context missing | [Incremental paths](../crates/core/src/growth.rs), [hardlink and fallback tests](../crates/core/tests/fsevents_incremental.rs) | Document interior fast paths, whole-artifact hardlink fallback, structural changes, and full-walk fallback. |
| FSEvents gives byte changes or identifies the writer | Incorrect | [FSEvents wrapper](../crates/core/src/fs_events.rs), [Apple event flags](https://developer.apple.com/documentation/coreservices/1455361-fseventstreameventflags/kfseventstreameventflagmustscansubdirs) | Events select where to remeasure. Swamp does not identify the writing process. |
| Swamp has a persistent daemon for scheduled observations | Incorrect | [Schedule implementation](../crates/core/src/schedule.rs), [CLI observer](../crates/cli/src/schedule.rs) | A LaunchAgent starts an observation process at intervals; the TUI separately owns a live watch. |
| Current rows and previous values support history comparisons | Verified with edit | [History storage and lookup](../crates/core/src/growth.rs) | Baseline selection uses the nearest retained observation. Avoid “exactly any instant” or “just a lookup” as a complexity guarantee. |
| Existing artifact byte updates are written to `current.parquet` | Verified with edit | [Fixed write condition](../crates/core/src/growth.rs), [new regression](../crates/core/tests/report_growth.rs) | Found and fixed a missing dirty flag. The regression failed before the fix and passes afterward. |
| Unchanged observations need no new artifact byte delta | Verified | [History tests](../crates/core/src/growth.rs), [sequential-update regression](../crates/core/tests/report_growth.rs) | Keep the claim scoped to unchanged values; metadata changes can still cause other writes. |
| A changed row is updated in place in Parquet | Incorrect | [Current-file writers](../crates/core/src/growth.rs) | Changed current datasets are rewritten. Reverse deltas reduce history duplication, not all write cost. |
| Every observation is transactionally durable | Unsupported | [Temporary-file writes and compaction](../crates/core/src/growth.rs) | Describe closing and renaming individual files; do not claim a transaction across files or all crash scenarios. |
| The history store is always a few hundred KiB | Unsupported | Previous README supplied a single-tree figure without a reproducible measurement procedure. | Explain representation and compression; omit the size guarantee. |
| All windows correspond to an exact historical snapshot | Misleading/context missing | [Nearest-time growth lookup](../crates/core/src/growth.rs) | Explain observation granularity and unavailable baselines. |
| History survives arbitrary worktree and remote renames | Incorrect | [Project/worktree IDs](../crates/core/src/consumers/projects.rs), [Git fallback identity](../crates/core/src/git.rs) | Worktree IDs are path-derived; changed paths or remote identity can split history. |
| Every root has an independent history namespace | Incorrect | [Volume storage, topology, and row replacement](../crates/core/src/growth.rs) | Prefer one common root per store; separate stores for independent roots on the same volume. |
| Independent consumer futures make all enrichment nonblocking | Misleading/context missing | [Dispatch loop](../crates/core/src/bus/mod.rs), synchronous calls in [GitHub](../crates/core/src/github.rs) and [Docker](../crates/core/src/docker.rs) | State current-thread polling, blocking-call limits, and separate worker concurrency. |
| Tracking and history form a serial chain | Incorrect | [Subscriptions](../crates/core/src/consumers/tracking.rs), [history](../crates/core/src/consumers/history.rs), [assembler](../crates/core/src/consumers/assemble.rs) | Both subscribe to growth annotation; final assembly waits for both. |
| A new fact source always needs only one file and one registration line | Misleading/context missing | [Event definitions](../crates/core/src/bus/mod.rs), [assembly gate](../crates/core/src/consumers/gate.rs) | New data contracts can require payload, gate, report, and interface changes. |
| Every interface computes and applies exactly the same view behavior | Incorrect | [CLI dispatch](../crates/cli/src/main.rs), [MCP filtering](../crates/mcp/src/main.rs), [TUI model](../crates/tui/src/model.rs) | Share the report pipeline; document interface-specific filtering and actions. |
| `report --filter` filters every CLI view and JSON | Incorrect | [CLI render dispatch](../crates/cli/src/main.rs) | It currently applies only to the root worktrees view. Remove ineffective artifact-filter recipes. |
| `report --no-observe` only reads the last cached report | Incorrect | [CLI request](../crates/cli/src/main.rs), [walk consumer](../crates/core/src/consumers/walk.rs) | It skips growth writes but can inspect the filesystem and consult caches. TUI cached mode is distinct. |
| `1w` works wherever a duration is accepted | Incorrect | [Filter parser](../crates/core/src/filter.rs), [growth parser](../crates/core/src/growth.rs) | Weeks work in filters; use `7d` for `--since`, configuration, expiry, and schedules. |
| Changing a growth filter's window recomputes the baseline | Incorrect | [TUI filter commit](../crates/tui/src/app.rs), [predicate evaluation](../crates/core/src/filter.rs), [growth consumer](../crates/core/src/consumers/growth.rs) | Filters use precomputed growth. Document the TUI label mismatch and use explicit CLI/MCP `since` for comparisons. |
| `growth < 500MB` means an increase smaller than 500 MB | Incorrect | [Growth predicates](../crates/core/src/filter.rs), [TUI filter](../crates/tui/src/filter.rs) | It selects shrinkage whose magnitude exceeds the threshold; include the required window clause. |
| Nine tools are advertised by MCP, with no grant-writing tool | Verified | Running `tools/list` and [tool-list integration test](../crates/mcp/tests/tool_list.rs) | List the nine tools. Approval remains a CLI/TUI operation. |
| Every MCP response carries identical history metadata | Incorrect | [MCP response handlers](../crates/mcp/src/main.rs) | State that metadata differs by tool. |
| The MCP authorization design prevents a shell-capable agent from authorizing | Misleading/context missing | [CLI approval](../crates/cli/src/main.rs), [MCP methods](../crates/mcp/src/main.rs) | Describe the interface boundary without claiming OS-level isolation. |
| Deletion is limited to folded artifacts | Stale/outdated | [TUI selection](../crates/tui/src/app.rs), [CLI/MCP proposal](../crates/core/src/actions.rs), [frame/action tests](../crates/tui/tests/frames.rs) | Document worktree/checkout/source selections and the project fallback. |
| All removals go to Trash | Incorrect | [Docker removal](../crates/core/src/docker.rs), [TUI actions](../crates/tui/src/actions.rs) | Filesystem paths go to Trash; Docker image/volume removal has no Trash copy. |
| Ignored means generated; tracked means remotely recoverable; untracked means no copy exists | Incorrect | [Tracking implementation](../crates/core/src/ignore.rs) does not inspect every possible backup or establish remote durability. | Explain what Git knows. State that swamp creates no backup. |
| Dirty or unpushed state universally blocks removal | Incorrect | [Action warnings](../crates/core/src/actions.rs), [TUI action terms](../crates/tui/src/actions.rs) | These facts inform confirmation and do not universally enforce a veto. |
| GitHub and Docker enrichment are always fresh | Incorrect | [GitHub TTL and keys](../crates/core/src/github.rs), [Docker cache](../crates/core/src/docker.rs) | Document six-hour GitHub TTL/tip matching and five-minute Docker cache; distinguish refresh-capable commands. |
| Reconciliation proves every byte of a volume is accounted for | Misleading/context missing | [Reconciliation types](../crates/core/src/report.rs), [walker](../crates/core/src/walk.rs) | Scope the claim to measured filesystem bytes and keep Docker accounting separate. |
| Source audits automatically run on any build | Incorrect | [Check script](../scripts/check.sh) | State what `scripts/check.sh` runs; plain `cargo build` does not run the full checks. |
| Growth is green, selection is reverse video, and only help uses an overlay | Stale/outdated | [TUI renderer](../crates/tui/src/ui.rs), [picker](../crates/tui/src/picker.rs) | Update DESIGN and the stored design snapshot to red growth, green shrink, dark selection, and help/filter overlays. |

## High-risk unresolved claims

No unsupported claim of exclusive history support, guaranteed speed, guaranteed recoverability, universal action checks, or crash-proof storage remains in the current guides. The implementation limits stated in the architecture guide still apply. This documentation review is not a comprehensive security or data-integrity audit.

## Claims requiring expert review

No external expert is needed to publish the bounded descriptions above. Stronger durability, filesystem coverage, or authorization guarantees would require a separate implementation review and tests for the stated guarantee.

## Source gaps

- The historical performance observations lack a controlled harness, hardware description, and repeated samples. They remain labeled as historical observations in the changelog.
- No tested minimum Rust version or cross-platform support matrix was established.
- No exhaustive competitor survey was performed. The direct counterexample is sufficient to reject the history-exclusivity claim.
- Text frame tests do not establish readability on every terminal, font, or palette.

## Corrections applied

The README now introduces the project model and history before installation and first use. Command and recovery details moved to the usage guide. The architecture guide explains mechanisms and costs with source links. PRODUCT, DESIGN, the design snapshot, the event-bus ADR, and the changelog were aligned with the implementation. Early Epic #1 notes are explicitly marked historical.

The artifact persistence issue was delegated to a Luna worker at the user's request and fixed with a regression test. It is recorded here as a correction, not presented as an unresolved product limitation.

## Verification

- `scripts/check.sh` passed: workspace tests, formatting, Clippy, source audits, and the script's additional source checks.
- The new artifact-history regression was observed failing without the fix and passing with it.
- All 116 local documentation links and anchors resolved; fenced blocks, 16 shell examples, and JSON examples passed syntax checks.
- The running MCP binary advertised the nine tools listed in the usage reference.
- GitHub release metadata confirmed the actual v0.4.0 archive names. Those historical assets remain unchanged; v0.5.0 is the first release built from the renamed source.
