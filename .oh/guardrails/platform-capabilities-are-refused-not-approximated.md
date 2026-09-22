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

AST audit `platform_capabilities_gate_their_backends` (`crates/source-audit/src/platform_audits.rs`), written in the derived-set form `review/REVIEW-STACK-3.md` §1 asks for — no file list, no function-name list, the workspace parsed once into definitions with call edges resolved to definitions rather than to names:

- **Capability queries are derived**: every definition anywhere in `crates/{core,cli,tui}/src` whose return type is a capability enum (`Scheduling`, `ContinuitySource`, `Support`, `Capability`). A renamed or new query joins the set on its own.
- **Asking is not refusing** (`GUARDRAILS_SPEC.md` §17 item 2): every call that may reach a query must flow into control flow; `let _ = scheduling();` fails.
- **The guard comes before the write, transitively.** The scheduling feature is derived from the CLI's own dispatch: the definitions the `Command::Schedule` arm reaches, minus those any other subcommand reaches. Every path from there to a mutation passes through a definition that asks — exactly, not through an ambiguous namesake — honours the answer, and asks *before* it writes. Helpers called before the asking statement are walked too. Mutations are an **inverted** set: every `std::fs::` call except a short list of reads, plus `File::create`, `OpenOptions::new` and `Command::new`, so an unlisted write fails closed.
- **The platform refusal is decided once**: the `RefreshRefusal` variants built by the one function that reads `ContinuitySource` (derived, not listed) may not be constructed by any other non-test function; an exhaustive `match` that only reads a refusal is exempt by shape, not by name.

Ten rejection fixtures in the mutation corpus under `crates/source-audit/tests/mutations/platform_capabilities_gate_their_backends/`: no check, check after the write, alias/rename (`use std::fs::write as emit`), discarded answer, second refusal decider, decider that stops reading the contract, writes moved into a helper, a write through `OpenOptions`, an entry point renamed in the CLI, and an unguarded namesake of the guarded `install`.

That last one is the finding worth keeping. The first version of the audit keyed its call graph by function name, reasoning that collapsing namesakes over-approximates and so fails closed. It fails *open* wherever the graph is subtracted: `work_counters::install` is called by every subcommand, so the name `install` counted as shared and `schedule::install` — the one function the rule exists for — was never walked. Five of the corpus's installers passed that way until edges resolved to definitions.

Target gating of `Os::current` and the `CAPABILITIES` table are not text checks: `scripts/platform-isolation.sh` proves the gating on each CI target's built binary (dependency graph, linkage and symbol table), and `platform_matrix_matches_docs` holds `CAPABILITIES` and `docs/platform.md` to each other in both directions.

Runtime halves, because an audit does not prove a refusal refuses:

- `schedule::tests::install_refuses_and_writes_nothing_where_there_is_no_scheduler` — no plist, no agents entry, no log directory.
- `schedule::tests::install_refuses_before_it_validates_the_interval`.
- `schedule::tests::uninstall_and_status_report_the_capability_not_an_empty_installation`.
- `platform::tests::linux_scheduling_refuses_with_a_reason_and_an_issue`.
- `fs_events::tests::a_kernel_without_persisted_history_says_so_rather_than_unsupported`.
- `platform::tests::a_live_watch_epoch_does_not_cover_time_before_the_watch_opened`.

## Limits

- **Dynamic dispatch.** The model resolves calls, not trait-object dispatch or function pointers. A future scheduling backend reached through a `dyn` trait is invisible to the walk; the runtime tests above (which assert the plist, agents and log directories are *empty*, not that a particular call was absent) are what catch it.
- **Shared helpers.** Code reachable from any other subcommand is subtracted from the scheduling feature, so a writing helper that `observe` also calls, invoked by `install` before its guard, is not walked. The subtraction uses exact call edges only, so ambiguity never widens it; the residual is a helper that genuinely is shared.
- **Methods.** A method call resolves to every definition with that name (methods are not aliased, and the receiver type is unknown to a syntax-level model). That over-approximates the walk, which fails closed — but an ambiguous method call cannot count as the *guard*: only a call that reaches exactly one definition, and that definition a query, does.
- **Statement granularity.** "Before" is by top-level statement. A write inside the same statement as the question (`if can() { write() }`) is treated as behind it, which is right for the shape guards are written in and wrong for a write placed in the refusal branch itself.
