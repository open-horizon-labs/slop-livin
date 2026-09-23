# 2026-09-23: porting the Linux track onto the capability gates

Ports stack/15 + stack/18 (Linux platform isolation, inotify watch,
`swamp collect`, `systemd --user` scheduling, the freedesktop Trash mover,
`/proc` occupancy) from before the capability gates existed
(`docs/architecture.md`, "Capability gates") onto `stack/23-build-adapters-on-gates`,
so every Linux `libc`/`std::fs`/`std::process` call lives inside
`crates/core/src/fs_gate` like the macOS backends already do.

## 1. What moved into the gate

New gate submodules, each the whole cohesive capability rather than only
its primitive calls (matching `fs_gate::destroy`'s and `fs_gate::spawn`'s
existing shape):

- `fs_gate::inotify` (target-gated, Linux only): the raw
  `inotify_init1`/`poll`/`read`/`inotify_add_watch`/`inotify_rm_watch`/`close`
  syscalls, moved out of `live_watch.rs` wholesale. `live_watch.rs` keeps
  only the portable `Kernel` trait, `Limits`, `RawEvent` and the
  `LiveTree` state machine, which stays testable with a fake kernel on
  either platform (as its doc comment already claimed); it re-exports
  `fs_gate::inotify` under its own name so existing callers
  (`inotify::Inotify::new()`, `inotify::limits()`) are unchanged.
- `fs_gate::procfs` (**not** target-gated: the fail-closed rules are
  pure over a fixture `/proc` tree and are exercised on both platforms
  by `occupancy.rs`'s own test module -- only `occupancy::probe_paths`'s
  call into it is Linux-only): `Creds`, `probe`, `self_pid`, and the
  fail-closed classification helpers.
- `fs_gate::systemd`: swamp's own `.service`/`.timer` unit files
  (`is_ours`, `write_unit`, `create_unit_dir`, `remove_unit`), plus
  thin wrappers (`current_uid`, `run`) so `systemd_user.rs` never names
  `fs_gate::sys`/`fs_gate::spawn` directly (the gate audit's per-module
  allow-lists didn't have `systemd_user` on them).
- `fs_gate::continuity`: the collector's checkpoint/sync-token atomic
  writes, `read_text`, and its `FileLock` (`flock` via
  `File::try_lock`/`try_lock_shared`/`unlock` -- the stable, non-`libc`
  standard-library API; only the *open* is gated,
  `fs_gate::sys::open_for_lock`/`open_for_lock_probe`).
- `fs_gate::fs_space` (moved from `platform/fs_space.rs`): the
  `statfs`/`statvfs` free-space backend; `platform::fs_space` re-exports
  it so every existing caller is unchanged.
- `fs_gate::destroy` gained `items_dir`/`write_trashinfo_sidecar`
  (Linux-only): `trash_move`/`Envelope::open` now lay out
  `files/`+`info/` and write the `.trashinfo` sidecar there, so the
  *same* proof-and-authorization-gated mover that already existed for
  macOS is the Trash backend on Linux too, rather than a second,
  ungated `platform::trash` module (deleted -- nothing outside this
  session's changes ever called it once `tui/actions.rs` and
  `core/actions.rs` were resolved back to their gate-model trash
  plumbing; see §3).
- `fs_gate::spawn` gained `Program::Systemctl`/`Program::Loginctl` with
  an allow-listed shape table (`show-environment`, `daemon-reload`,
  `enable`/`restart`/`disable --now` on swamp's own unit names only,
  `stop` on the service unit only, `show` with a fixed property
  allow-list; `loginctl show-user` with this process's own uid only).
  `systemd_user::RealSystemctl` runs through it instead of
  `std::process::Command` directly.
- `fs_gate::sys` gained `current_uid`, `is_enospc`,
  `install_stop_signal_handlers` (the collector's SIGINT/SIGTERM
  handler -- `extern "C"`, `libc::signal` -- moved wholesale since the
  handler itself is `unsafe`), `open_for_lock`/`open_for_lock_probe`.

## 2. The gate audit's per-module allow-lists

`gate_paths_only_inside_gates` (`crates/source-audit/src/rules/gate.rs`)
restricts several existing gate submodules (`fs_gate::sys`,
`fs_gate::read::read_owned_string`, `fs_gate::read_dir`,
`fs_gate::spawn::run`, and every individual `Program::*` variant) to a
named allow-list of calling modules. Added: `live_watch` to `WALKERS`
(its directory registration is a real traversal) and to the `sys`
allow-list (`is_enospc`); `systemd_user` to the new
`Program::Systemctl`/`Program::Loginctl` entries. Everywhere else
(`continuity.rs`), rather than widen an existing allow-list, the new
`fs_gate::continuity`/`fs_gate::systemd` submodules grew their own
narrow wrapper functions (`read_text`, `write_atomic`, `stop_on_signals`,
`current_uid`, `run`) so the non-gate caller never names a *different*
gate submodule's path at all -- the same shape the existing store
modules already have for `fs_gate::store`.

`std::os::unix::fs::MetadataExt` is banned by name (no allow-list
exceptions for that one); `crate::fs_gate::MetadataExt` (the gate's own
re-export of the same trait) is not the same path and is what
`continuity.rs`/`live_watch.rs` import instead.

## 3. Conflicts resolved by keeping the gate-model side

Every merge conflict between the Linux track (pre-gate) and
`stack/23`'s gate model followed one shape: the Linux track re-derived
something the gate model had already built as a proof-and-authorization-
gated primitive (`fs_gate::destroy::trash_move`/`Envelope` vs. the old
`platform::trash::Target`/`move_item`; `fs_gate::spawn::run` vs. a raw
`Command`). Resolution was always "keep the gate side, delete the
duplicate," in `actions.rs`, `cargo_cleanup.rs`, `tui/actions.rs`,
`tui/app.rs` (one leaked call site fixed by hand after a `git apply
--3way` clean-merge silently substituted a signature it shouldn't have),
`fs_events.rs` (kept the already-extracted `fs_events/macos.rs` module
and the `watch_pending`/`PendingWatch` batching the gate model added,
folding in `watch_excluding`'s exclusion parameter and `live_watch`
dispatch) and `occupancy.rs` (kept the gated `lsof_probe`, added the
Linux `Procfs` dispatch arm calling `fs_gate::procfs::probe`).

`crates/source-audit/{linux_audits.rs,platform_audits.rs}` (the old
`syn` call-graph audits) were deleted; their fixtures were reclassified
(see the three `.oh/guardrails/*.md` files this touched:
`trash-backend-owns-every-move.md`, `occupancy-gaps-are-unknown-never-free.md`,
`platform-capabilities-are-refused-not-approximated.md`) rather than
kept pointing at code that no longer exists.

## 4. What is scoped down from the original design

- **No atomic `renameat2(RENAME_NOREPLACE)`.** `trash_move` does a
  plain existence check immediately before `rename(2)`: a narrower
  (TOCTOU-able) guarantee than the original design's atomic syscall.
- **No per-mount `.Trash-$uid` fallback.** A trash root on another
  filesystem is refused outright (`Envelope::open`'s `same_device_as`),
  not redirected to that mount's own trash directory.
- **No `.trashinfo` reserved with `O_EXCL` before the move**; it is
  written best-effort immediately after a successful rename.
- **`StoreDir::resolved()` is unchanged**: still `$SWAMP_DIR`, else
  `$HOME/.local/share/swamp`, falling back to `.` with no `HOME` set.
  The original Linux track's `platform::data_dir` (refuses with no
  `HOME`, honours `$XDG_DATA_HOME`) was deliberately not ported: it
  would have been a second resolver for where swamp's state lives,
  and the brief this chunk followed named `fs_gate::store::StoreDir`
  as the one place that decides. Documented as a known regression
  in `CHANGELOG.md`, not silently dropped.
- **CLI/agent-action-layer Linux ergonomics were not extended.** Per a
  mid-session correction, the CLI/agent propose-approve-execute path is
  slated for removal in a separate chunk; this session kept it
  compiling wherever the port touched it but did not add Linux-specific
  polish there.

## 5. What is not done or not verified

- No Linux CI run has executed any of this. Cross-compiling with a C
  toolchain from the macOS host this work was done on is not available
  (`x86_64-linux-gnu-gcc` is not installed, and `cross` has no local
  Docker daemon to fall back to), so verification is `cargo check
  --target x86_64-unknown-linux-gnu` (type-checks, catches the same
  class of error a `cfg(target_os = "linux")` block would hide from a
  native macOS build) plus native `cargo check`/`clippy`/`test`/
  `scripts/check.sh` on macOS. The actual inotify/procfs/systemd code
  paths have run nowhere.
- **Update, later the same session:** `scripts/check.sh` ran clean
  three times (with `SWAMP_TARGET_DIR` set, without it at all, and
  after fixing one fixture below). `scripts/check-full.sh` itself
  stopped at its `compile-fail` step, but on eight cases wholly
  unrelated to this port (`bus_stage_has_no_production_test_constructor.rs`,
  `fs_events_testing_is_not_in_production.rs`,
  `human_confirmation_names_what_was_confirmed.rs`,
  `metadata_does_not_follow_by_default.rs` and four more): `trybuild`
  reports a wording mismatch against the checked-in `.stderr` snapshot
  ("no function or associated item named" vs. "no associated function
  or constant named" -- the same rustc error, reworded between compiler
  versions), not a behavior change. Confirmed pre-existing: `git diff
  stack/23-build-adapters-on-gates..HEAD -- crates/core/tests/compile_fail/
  crates/core/src/bus.rs` is empty; neither this port nor any commit on
  this branch touched those fixtures or the APIs they test. Not fixed
  here (regenerating eight trybuild snapshots against this toolchain is
  independent of the Linux port). Run separately (past that step) and
  passing: `mutation_sweep.rs`'s two ignored tests (892s -- every
  fixture, seed and operator-derived variant, including this session's
  `trash_backend_owns_every_move`/`platform_capabilities_gate_their_backends`
  additions) and `reviewer_cost_measurement_stack3.rs`'s two ignored
  tests (54s, including `spawn_oracle_covers_every_program_the_gate_can_run`,
  which covers the new `Program::Systemctl`/`Program::Loginctl`
  variants).
- One fixture needed a real fix, found by the second `check.sh` run:
  `systemd_user.rs`'s `exec_arg` control-character test used the
  literal string `rm -rf` (proving a newline-injected directive is
  refused, not escaped), which `check.sh`'s own destructive-shortcut
  grep matches regardless of context; reworded to prove the same
  refusal without the banned substring.
- `docs/platform.md`'s prose was patched where it named the deleted
  `platform/trash.rs`/`occupancy::procfs_probe` paths and where it
  overclaimed `renameat2`/`O_EXCL`/cross-device fallback; it has not
  been re-read end to end for other drift, and
  `platform_matrix_matches_docs.rs` (which holds the `CAPABILITIES`
  table against this file in both directions) has not been run.
