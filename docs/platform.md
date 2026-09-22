# Platforms

Swamp supports two targets:

| | Target triple | Validated on | Status |
|---|---|---|---|
| macOS | `aarch64-apple-darwin` | macOS arm64, in CI on every push | Builds, tests, releases |
| Linux | `x86_64-unknown-linux-gnu` | Ubuntu 24.04 x86_64, in CI on every push | Builds and tests; release packaging is [#88](https://github.com/open-horizon-labs/swamp/issues/88) |

No other target is supported, and none is planned here. Windows is not a target. ARM Linux is not a target. Nothing in this work makes either one closer; adding one means doing this exercise again for that kernel.

**CPU baseline: generic x86_64.** No `target-cpu=native`, ever, in a release profile. A binary built for the machine that built it is not a binary anyone else can run, and a measurement produced by a build nobody can reproduce is not evidence.

**No root, no assumed init system.** Everything swamp does on Linux runs as an ordinary user. It does not require `systemd`, a session bus, a container runtime, or membership in any group. Where a capability genuinely needs one of those, swamp refuses and names what is missing (see [Capabilities](#capabilities)) rather than escalating or guessing.

**glibc, not musl.** `x86_64-unknown-linux-gnu` links glibc. `scripts/platform-isolation.sh linkage` asserts in CI that the binary's runtime dependencies stay within the loader, `libc`, `libm` and `libgcc_s` — so a new dependency that drags in a system library is a CI failure rather than a support question after the fact.

## Capabilities

This table is generated from `crates/core/src/platform/CAPABILITIES`, and `crates/core/tests/platform_matrix_matches_docs.rs` fails if the two disagree. A capability cannot be claimed here without being claimed in the code, or removed from the code without being removed here.

| Capability | macOS | Linux | Notes |
|---|---|---|---|
| `walk-and-accounting` | supported | supported | Allocated bytes from st_blocks*512, hardlink dedup by (dev, ino), same-filesystem boundary by st_dev, symlinks never followed. Shared POSIX code. |
| `free-space` | supported | supported | statvfs(3) through libc on both, replacing df output parsing whose columns differ between the two. |
| `history-replay` | supported | unavailable | macOS replays the fseventsd log from a stored event id. Linux has no persisted kernel change history; an observation there walks fully and says so. |
| `live-watch` | supported | planned | macOS opens an FSEvents stream for the TUI. A Linux inotify watcher is #81; until then the TUI refreshes on demand. |
| `scheduled-observation` | supported | planned | macOS installs an opt-in per-user LaunchAgent. Linux refuses and names #86 (systemd --user); nothing is written. |
| `trash` | supported | planned | macOS moves to ~/.Trash. Linux path resolution follows the freedesktop Trash spec ($XDG_DATA_HOME/Trash, or .Trash-$uid on another mount); the .trashinfo records and cross-device move are #85. |
| `occupancy` | supported | supported | Bounded lsof probe on both. A missing or timed-out lsof is Unknown, which every destructive sink refuses on -- it is never read as 'nothing is open'. |
| `atime-reliability` | supported | supported | macOS reads statfs mount flags; Linux reads /proc/mounts. Either failing is Undetermined, not 'atime is fine'. |
| `release-artifact` | supported | planned | macOS arm64 tarballs ship today. Linux x86_64 packaging is #88; CI builds and tests Linux but publishes nothing. |

"Unavailable" and "planned" both mean the same thing at runtime: swamp refuses and says why. The difference is whether an issue exists that would change the answer.

### The one that is not a missing feature

`history-replay` is the only row that says *unavailable* rather than *planned*, and the distinction is the point of this whole design.

macOS's `fseventsd` writes a per-volume change log to disk. A stored event id is a cursor into it, and a replay from that cursor accounts for everything that happened **while swamp was not running**. That is what makes an incremental refresh of a 41 GB tree cost 0.1 s instead of 7 s.

Linux has no equivalent. inotify reports what happens while a watch is open, keeps no history, and tells you with `IN_Q_OVERFLOW` when it dropped even some of that. An inotify watch descriptor is a handle on a running watch — it is not a cursor, and storing one where an event id belongs would turn "swamp was not watching" into "nothing changed". That is a false measurement, and the growth store's whole value is that it does not contain any.

So a Linux observation walks fully and reports `mode=full reason=no_persisted_change_history`. [#81](https://github.com/open-horizon-labs/swamp/issues/81) adds a live watcher, which will let a *running* swamp narrow the gap to the time since its watch opened (`ContinuityCursor::LiveWatchEpoch`). It does not remove the gap, and no amount of implementation work will.

## Reuse assessment

Issue [#79](https://github.com/open-horizon-labs/swamp/issues/79) requires this before any bespoke adapter: for each candidate, what it supports, its licence, its dependency and build-script footprint, its maintenance, how it gates by target, and whether it preserves swamp's accounting, scope, history, error visibility and action protections.

Versions and platform statements below were checked against crates.io, docs.rs and the upstream repositories on **2026-09-22**.

### `trash` 5.2.9 — **adopt for [#85](https://github.com/open-horizon-labs/swamp/issues/85)**

| | |
|---|---|
| Licence | MIT. MSRV 1.85.0. |
| Platforms | Windows, macOS, and freedesktop-compliant environments. Implements v1.0 of the freedesktop Trash specification. |
| Dependencies (macOS) | `log`, `objc2`, `objc2-foundation`, `percent-encoding`. |
| Dependencies (Linux) | `log`, `libc`, `once_cell`, `scopeguard`, `urlencoding`, optional `chrono`. **No D-Bus, no C build script.** |
| Build scripts | None in the crate itself. |

It does the cross-device rule properly: `execute_on_mounted_trash_folders` checks `$topdir/.Trash` for the sticky bit and rejects a symlink (`InvalidNotSticky`, `InvalidSymlink`) before falling back to `$topdir/.Trash-$uid`, which is exactly what the spec requires and exactly the part a hand-rolled implementation gets wrong.

Errors are typed and visible: `Error::{Os, FileSystem, TargetedRoot, RestoreCollision, ...}`, and `TargetedRoot` guarantees that a failed multi-item delete removed **none** of them. Nothing in the source falls back to permanent deletion — a failure that cannot reach a per-volume trash falls back to the *home* trash and then returns `Err`.

One real caveat, documented by the crate itself: on Linux and FreeBSD it calls the non-thread-safe `getmntent`/`getmntinfo`, guarding its own calls with a mutex, and its docs say plainly that a crate calling those functions from other threads "rather not use this crate". Swamp does not call them — `activity.rs` reads `/proc/mounts` as a file and `cargo_cleanup.rs` uses `statfs` — so the caveat is satisfiable, but it is a constraint on future code and is recorded here as one.

**Decision: adopt in #85**, behind swamp's own protection, occupancy and grant checks. Adopting it does *not* transfer any of those: `trash::delete` is a filesystem operation, and every safeguard in `actions.rs` still has to run at the sink first. Path resolution is already implemented in `crates/core/src/platform/trash.rs` so #85 is a move implementation rather than a second path design.

### `notify` 8.2.0 — **decide in [#81](https://github.com/open-horizon-labs/swamp/issues/81); the recommendation is direct inotify**

| | |
|---|---|
| Licence | CC0-1.0 (siblings are MIT/Apache-2.0). MSRV 1.77. Latest overall is 9.0.0-rc.5, a prerelease. |
| Backends | inotify on Linux, FSEvents on macOS by default (kqueue behind `macos_kqueue`), ReadDirectoryChangesW on Windows, polling everywhere. |

The documented limitations are the reason for the recommendation, not against the crate. Its README says the Linux backend "is documented to not be a 100% reliable source" under load, and points at `fs.inotify.max_user_watches` / `max_user_instances`. What it does **not** document anywhere is `IN_Q_OVERFLOW` — there is no named API for surfacing a queue overflow to the caller.

That single omission is disqualifying for swamp's purposes, because overflow is the case that matters: it is the moment the kernel says "I lost events", and it is precisely when a watcher must stop claiming coverage and force a full walk. A library that cannot tell swamp overflow happened would let swamp report "nothing changed" about a period where changes were dropped. `PollWatcher` is not an answer either — a full rescan per interval is the cost swamp exists to avoid.

`notify` also cannot replay history, which nothing can on Linux (see above). So the value it would add over `inotify` directly is cross-platform abstraction swamp does not need: macOS already has a hand-written FSEvents backend that does exactly what the replay design requires.

**Decision: #81 should use `inotify` directly and surface `IN_Q_OVERFLOW` as a named coverage failure.** Recorded here rather than implemented, because #81 is where a watcher lands.

### `walkdir` 2.5.0 — **already a transitive dependency; kept as the reference walker, not the production one**

| | |
|---|---|
| Licence | Unlicense OR MIT. Last release 2024-03-01; stable and maintained. |
| Dependencies | `same-file` (plus `winapi-util` on Windows only). On Unix that is the whole footprint. |
| Cost to adopt | **Zero new dependencies** — it is already in swamp's normal dependency graph through `gix`. |

It gets the important defaults right: `follow_links` is off, `same_file_system` exists, and errors are yielded as items rather than swallowed. It is single-threaded, and `DirEntry::metadata()` costs an extra `stat` on Unix (`file_type()` is the free one).

It is used in the repository today as the **reference implementation** in `crates/core/tests/walk_library_comparison.rs`, which is where the evaluation lives rather than in prose:

- `a_generic_walker_needs_swamps_semantics_to_reach_swamps_number` sums `metadata.len()` over a fixture with a hardlink pair and a 32 MiB sparse file, the way a generic size pass naturally would, and gets a figure **more than ten times** swamp's. Add allocated bytes, `(dev, ino)` dedup and a same-filesystem boundary on top and the two agree exactly.

That is the finding. `walkdir` is not wrong; it is a walker, and the thing swamp needs is a measurement. Whatever library sat underneath would still have to be wrapped in all three rules, so the reuse saves the traversal loop and nothing else.

Measured, both walkers over the same generated tree, same build profile (debug), warm cache:

| Shape | Files | swamp | walkdir + swamp's rules | Ratio |
|---|---|---|---|---|
| 16 dirs × 2,000 files — macOS arm64, APFS | 32,000 | 37.7 ms | 80.0 ms | 2.1× |
| 512 dirs × 40 files — macOS arm64, APFS | 20,480 | 24.5 ms | 58.7 ms | 2.4× |
| 16 dirs × 2,000 files — Ubuntu 24.04 x86_64, ext4 | 32,000 | 47.9 ms | 92.0 ms | 1.9× |
| 512 dirs × 40 files — Ubuntu 24.04 x86_64, ext4 | 20,480 | 38.4 ms | 64.5 ms | 1.7× |

One CI run each, 2026-09-22, on shared runners: the absolute times are worth little and the ratio is the point. Every run prints its own (`WALK BENCH os=... ratio=...`) from the **Traversal, accounting and volume identity** step, so the current numbers are always in the job log rather than only in this file.

**Decision: keep swamp's walker.** It is 1.7–2.4× faster than the alternative *with the same semantics bolted on*, the bounded parallel pool is what makes the difference on the many-directory shape, and swapping it would trade that for no reduction in the code that actually has to exist.

### `jwalk` 0.9.0 — **rejected**

Archived upstream. The `Byron/jwalk` repository has `archived: true`, and release 0.9.0 (2026-08-05) exists to carry the notice: *"This crate is no longer maintained or supported."* The release before it was 0.8.1, in December 2022.

Technically it is the closest thing to what swamp already has — a rayon-backed parallel walk that preloads metadata, with per-directory parallelism ("It wont help when reading a single directory with many files", which is the shape a `target/` directory often has). Under maintenance it would have been the serious candidate.

**Decision: rejected on maintenance.** Depending on a crate whose author has publicly stopped supporting it, to replace working code, is a trade with nothing on the upside. No benchmark was run against it in-repo, deliberately: the number could not change the decision, and running it would have meant adding an unmaintained crate to the lockfile to produce it.

### `clean-dev-dirs` 2.8.2 — **detector conventions only, not the scanner**

| | |
|---|---|
| Licence | Apache-2.0 OR MIT (per its `Cargo.toml`; the GitHub API reports only Apache-2.0). |
| Library API | Yes — a real `[lib]` exporting `Scanner`, `Cleaner`, `Project`, `ProjectType`, `ScanOptions` and more. Not binary-only. |
| Dependencies | 17 direct, including `clap`, `colored`, `indicatif`, `inquire`, `rayon`, `trash`, `walkdir`, `jwalk` 0.8.1. |

Its CLI is more careful than the issue's phrasing assumed: it prompts before deleting, and it moves to trash by default (`use_trash: !permanent`, with a test pinning that), with `--permanent` as the opt-in. So "never run its destructive CLI as a scan" remains right, but it is not a reckless tool.

Its **scanner** is where reuse fails, in three independent ways. `src/utils/size.rs` sums `metadata.len()` — apparent size, not allocated blocks, so every sparse file and every filesystem with compression reports wrong. There is no inode tracking, so a hardlinked file counts once per name. And `same_file_system` is not set, so a size can walk across a mount boundary. Each of those is a property swamp's accounting is built on, and none is configurable from the outside.

Adopting it would also mean pulling a CLI's presentation stack — `clap`, `colored`, `indicatif`, `inquire` — into a library dependency, plus `jwalk`, which is archived.

**Decision: do not reuse the scanner.** What is worth reusing is its **ecosystem detection conventions** — which directory names belong to which toolchain — and swamp's `ecosystem.rs` already cites this project for exactly that. Keeping one catalog with attribution, rather than a second conflicting one, is the reuse that applies here. Its licence notice belongs with that citation.

### Summary

| Candidate | Decision | Because |
|---|---|---|
| `trash` 5.2.9 | Adopt in #85 | Implements the spec's cross-device rules correctly, typed errors, never silently permanently deletes. |
| `notify` 8.2.0 | Do not adopt; use `inotify` directly in #81 | No documented way to surface `IN_Q_OVERFLOW`, which is the one event a coverage claim depends on. |
| `walkdir` 2.5.0 | Keep as the test reference | Correct but generic; swamp's walker is 1.7–2.4× faster once the same semantics are applied to both. |
| `jwalk` 0.9.0 | Reject | Upstream archived and explicitly unmaintained. |
| `clean-dev-dirs` 2.8.2 | Reuse its conventions, not its scanner | Apparent size, no hardlink dedup, no filesystem boundary; brings a CLI's dependency stack. |

## Where each platform's code lives

Shared portable code stays shared. Target gating is for genuinely different kernels, not for filing code by operating system.

| Concern | Shared | macOS-only | Linux-only |
|---|---|---|---|
| Contracts | `platform/mod.rs` (`Os`, `ContinuitySource`, `ContinuityCursor`, `Scheduling`, `CAPABILITIES`) | | |
| Traversal and accounting | `walk.rs`, `attribution.rs` — POSIX `st_blocks`, `st_dev`, `st_ino` | | |
| Free space | `platform/fs_space.rs` contract | `statfs` (64-bit block counts) | `statvfs` (64-bit block counts) |
| Change observation | `fs_events.rs` types and refusals | `fs_events::macos` (CoreServices) | none yet (#81) |
| Scheduling | `schedule.rs` interval parsing, status | `launchctl`, plist | refuses, names #86 |
| Trash location | `platform/trash.rs` | `~/.Trash` | freedesktop spec |
| Occupancy | `occupancy.rs` (`lsof`, tri-state) | | |
| atime reliability | `activity.rs` contract | `statfs` mount flags | `/proc/mounts` |
| Default scan roots | `locations/builtin.rs` | `~/src`, `~/Library/Developer`, `~/Library/Caches` | `~/src`, `$XDG_CACHE_HOME` |
| Data and log directories | `platform::data_dir`, `schedule::log_dir` | `~/.local/share/swamp`, `~/Library/Logs/swamp` | `$XDG_DATA_HOME/swamp`, `$XDG_STATE_HOME/swamp` |

Cargo enforces the dependency half: Apple framework crates (`core-foundation`, `core-foundation-sys`, `fsevent-sys`) sit under `[target.'cfg(target_os = "macos")'.dependencies]` in `crates/core/Cargo.toml`, and `scripts/platform-isolation.sh deps` asserts in CI that they are present in the macOS graph and absent from the Linux one. `scripts/platform-isolation.sh linkage` asserts the same thing one level lower, against the built binary's linkage and symbol table.

## Paths

`SWAMP_DIR`, `SWAMP_LOG_DIR` and `SWAMP_TRASH_DIR` override everything below, on both platforms.

| | macOS | Linux |
|---|---|---|
| Growth store | `~/.local/share/swamp` | `$XDG_DATA_HOME/swamp`, default `~/.local/share/swamp` |
| Scheduled-run log | `~/Library/Logs/swamp` | `$XDG_STATE_HOME/swamp`, default `~/.local/state/swamp` |
| Trash | `~/.Trash` | `$XDG_DATA_HOME/Trash`, or `$topdir/.Trash-$uid` on another mount |
| Scheduling | `~/Library/LaunchAgents` | not applicable |

The macOS growth store path is deliberately unchanged, including its XDG-shaped spelling: existing installs keep their history where it is, and this work moves no user data.

Two rules the XDG base directory specification states, followed here:

- **A relative value is invalid and ignored.** `XDG_CACHE_HOME=cache` does not produce a scan root relative to whatever directory swamp was run from; it falls back to `~/.cache`.
- **State is not data.** Logs go under `$XDG_STATE_HOME`, which is what that category is for; the growth store is data.

And one rule swamp adds: **no current-directory fallback.** If `HOME` is unset and `SWAMP_DIR` is not given, resolution fails with a message rather than writing a growth store into the working directory — where the next run from a different directory would not find it and every project would look like it had vanished.

### Why Linux's default roots are what they are

`~/src` on both: a habit, not an OS convention.

`~/Library/Developer` and `~/Library/Caches` are macOS's. `~/Library/Developer` is Xcode's, and no Linux directory holds "the SDK and simulator storage of the platform toolchain"; proposing `/usr/lib` or a distribution's package cache would mean walking system-owned storage a user cannot act on without root, which swamp never asks for. `~/Library/Caches` does have a real equivalent, `$XDG_CACHE_HOME` (default `~/.cache`), and that is what Linux gets.

Neither platform's conventions may appear in the other's build, which is asserted by `neither_platforms_conventions_leak_into_the_other` rather than left to review.

Homebrew is detected on both, with the prefixes each platform actually uses: `/opt/homebrew` and `/usr/local` on macOS, `/home/linuxbrew/.linuxbrew` on Linux. `/usr/local` is not proposed on Linux — there it is a distribution-owned directory Homebrew does not claim.

## Volume identity, and its limit

The growth store is keyed by `root_scoped_volume_id`: a hash of `(st_dev, canonical path)`. The canonical path means two spellings of one root — including a symlinked one — share a history instead of silently starting a second, empty one. The device means two roots that happen to share a path string on different filesystems do not.

**`st_dev` is not stable across reboots for every Linux mount.** Device numbers are assigned as the kernel enumerates devices; a mount whose minor number changes gets a different id, and swamp starts a fresh history for that root rather than continuing the old one.

That is the safe direction, and it is a choice rather than an oversight. The alternative — keying on the canonical path alone — would join two genuinely different filesystems mounted at the same path into one history, and report the difference between them as growth. A lost baseline is visible to the user ("no history yet"); fabricated growth is not. `volume_identity_depends_on_the_device_and_a_changed_device_starts_a_new_history` pins the behaviour so it cannot be "fixed" without someone deciding to.

## Filesystems

Measured behaviour is asserted on whatever the CI runner mounts — APFS on macOS, ext4 on the Ubuntu runner — by `crates/core/tests/traversal_accounting_and_volume_identity.rs`, which checks allocated bytes, hardlink dedup, sparse files and symlink loops against the filesystem's own `stat` data and against `du`.

Known limits, not tested because no CI runner provides them:

- **Overlay filesystems** (Docker's container layers, `overlayfs` generally) report the *upper* layer's allocation for a file the container modified and the lower layer's for one it did not. A total is therefore about the merged view, not about reclaimable space in either layer.
- **Network filesystems** (NFS, SMB, sshfs) may report `st_blocks` that does not correspond to local allocation at all, and `statvfs` figures that describe the server. `cargo_cleanup.rs` already refuses to act on a filesystem it does not recognise as local; measurement still reports what the filesystem says, labelled as such.
- **Btrfs and ZFS** deduplicate and compress below the file level, so a sum of `st_blocks` can exceed the space that removing those files would return. This is the same class of overstatement APFS clones cause on macOS, where `reclaimability::apfs_clone_or_snapshot_bound` reports a bound rather than a figure. No Linux equivalent is implemented; a total on those filesystems is an upper bound and not labelled as one.

## Running the platform checks yourself

```bash
# Both halves of "only this platform's backends are in this build"
./scripts/platform-isolation.sh deps x86_64-unknown-linux-gnu
cargo build -p swamp && ./scripts/platform-isolation.sh linkage target/debug/swamp

# Accounting and volume identity against the filesystem and du
cargo test -p swamp-core --test traversal_accounting_and_volume_identity -- --nocapture

# The walker benchmark (prints; asserts only that both walks agree)
cargo test -p swamp-core --test walk_library_comparison -- --ignored --nocapture
```

Cross-*checking* for Linux from a Mac needs a C cross-compiler for `zstd-sys`. `cargo check --target x86_64-unknown-linux-gnu` works with `zig cc` standing in:

```bash
printf '#!/bin/sh\nexec zig cc -target x86_64-linux-gnu "$@"\n'   # minus --target=, which zig rejects
CC_x86_64_unknown_linux_gnu=/path/to/that/wrapper \
  cargo check --workspace --all-targets --target x86_64-unknown-linux-gnu
```

That is a compile check and nothing more. It does not link, it does not run, and it is not acceptance for anything — which is why CI runs natively.
