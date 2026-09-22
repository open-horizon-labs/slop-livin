---
id: platform-capabilities-are-refused-not-approximated
severity: hard
statement: "A capability the running platform does not have is refused with a named reason and leaves no state behind; it is never approximated by doing the other platform's thing badly."
outcome: disk-growth-by-project
audit: platform_capabilities_gate_their_backends
---

## Rationale

Every way this went wrong on Linux before the contract existed had the same shape: the code did the macOS thing, on a platform where the macOS thing means nothing, and reported success.

`swamp schedule --every 1h` wrote a LaunchAgent plist into a `~/Library/LaunchAgents` that no daemon on the machine reads, called a `launchctl` that does not exist, and printed "Scheduled observation every 1h". The user then believes a baseline is being recorded. It is not, and they find out the first time they ask a growth question — which is the moment the tool was supposed to be useful.

Free space came from `df -k`'s fourth whitespace-separated field. That is the "Available" column on macOS. GNU coreutils prints a different header and wraps a long device name onto a second line, so the same read can return a capacity percentage or nothing.

The most consequential one is continuity. `RefreshRefusal::UnsupportedPlatform` says a backend is missing — true on a platform nobody has written one for, and an invitation to write it. On Linux it is the wrong statement: the kernel keeps no change history to replay, and no implementation work changes that. #81's live watcher narrows the gap to the time before a watch opened; it does not close it. Reporting "unsupported" would promise a Linux user an incremental refresh that can never arrive. Worse, an inotify watch descriptor stored where an FSEvents event id belongs would make "swamp was not watching" read as "nothing changed" — a false measurement in a store whose whole value is that it contains none.

A refused capability is visible. An approximated one is not.

## Detection

AST audit `platform_capabilities_gate_their_backends`:

- `platform/mod.rs` declares a `CAPABILITIES` table and `Os::current` is target-gated, so neither platform's answer compiles into the other's build.
- Every definition of `schedule::install` consults the platform's scheduling capability, **honours** the answer (a discarded result fails, per `GUARDRAILS_SPEC.md` §17 item 2), and consults it **before** any write. A refusal after a write is not a refusal: it leaves the state behind and reports failure.
- `RefreshRefusal::UnsupportedPlatform` and `::NoPersistedChangeHistory` are named in exactly one function, `fs_events::platform_refusal`, which reads `platform::ContinuitySource`. A second copy can disagree with the first.

Six rejection fixtures in the mutation corpus under `crates/source-audit/tests/mutations/platform_capabilities_gate_their_backends/`, including the alias/rename variant (`use std::fs::write as emit`, which defeats a text-matching rule) and the discarded-result variant (`let _ = scheduling();` first, in the right place, answer thrown away).

Runtime halves, because an audit does not prove a refusal refuses:

- `schedule::tests::install_refuses_and_writes_nothing_where_there_is_no_scheduler` — no plist, no agents entry, no log directory.
- `schedule::tests::install_refuses_before_it_validates_the_interval`.
- `schedule::tests::uninstall_and_status_report_the_capability_not_an_empty_installation`.
- `platform::tests::linux_scheduling_refuses_with_a_reason_and_an_issue`.
- `fs_events::tests::a_kernel_without_persisted_history_says_so_rather_than_unsupported`.
- `platform::tests::a_live_watch_epoch_does_not_cover_time_before_the_watch_opened`.

## Limits

The resolver sees calls, not trait-object dispatch: a future scheduling backend reached through a `dyn` trait would not be matched by name, and the runtime tests are what would catch it. The audit also knows only the three filesystem writes in `INSTALL_WRITES`; a fourth spelling (say, a helper that itself writes) added to `install` would pass it, which is why the runtime test asserts the *directories are empty* rather than that a particular call was not made.
