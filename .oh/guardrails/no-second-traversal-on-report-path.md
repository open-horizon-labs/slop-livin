---
id: no-second-traversal-on-report-path
severity: hard
statement: "The ordinary report path traverses directories only in the folded walk. External units take their bytes from folded rows; agent adapters take directory structure from those rows or from the capped locations::shallow_list; nothing on the report path re-walks a tree the walk already measured."
outcome: disk-growth-by-project
audit: no_second_traversal_on_report_path
---

## Rationale

P1 in the 2026-09-21 review: `external.rs` recursively re-sized every
external root on *every* call, `agents/mod.rs` implemented a second
recursive traversal, and each adapter repeated it and re-read session
headers every time. The validation session explicitly deferred
incrementality and argued bounded header reads were enough. They are
not: the handoff's requirement is that unchanged work scales with roots
and changed containers, not with all files, and a 300-session warm
fixture does not test an unchanged large tool home.

## Detection

`fs::read_dir`, `read_dir(`, `walkdir`, `jwalk` and `resize_artifact*`
may appear only in the allow-listed traversal modules (`walk.rs`,
`fs_events.rs`, `attribution.rs`, `cargo_artifacts.rs`,
`cargo_cleanup.rs`, `recheck.rs`, `folded_measurement.rs`, and the
store/git/scan helpers). They are forbidden in `external.rs`,
`consumer_wiring.rs`, `external_associations.rs`,
`toolchain_declarations.rs`, `agents/**` and `locations/**`.

`cargo_cleanup.rs` and `recheck.rs` are allow-listed because they list
the members of one already-selected group at action time — a recheck of
an exact selection, not an observation pass.

One further exemption, and it is a `(file, function)` pair rather than a
whole file: `locations/mod.rs::shallow_list`. The guardrail spec carved
this out itself — "detector modules that genuinely need one shallow
listing must go through a bounded helper `locations::shallow_list` which
is itself allow-listed and capped" — because some layouts really do
require enumerating exactly one level (a version manager's `versions/`,
a tool home's top-level entries). Everything that used to call
`read_dir` in `agents/**` and `locations/**` now calls it, adapters
reach it only through `agents::IdentifyCtx`, and it is sorted, capped at
`SHALLOW_LIST_CAP`, never recursive, and refuses to follow a symlink
into another tree.

The exemption is deliberately narrow in two ways the audit enforces:

- it is scoped to that one function, so a second traversal added
  *beside* it in the same file still fails (mutation test:
  `the_one_bounded_lister_is_exempt_but_a_sibling_in_the_same_file_is_not`);
  and
- the audit requires `SHALLOW_LIST_CAP` to still exist, so the
  exemption cannot outlive the bound that earns it (mutation test:
  `a_bounded_lister_that_lost_its_cap_is_rejected`).

`agents/mod.rs::folded_bytes` moved to
`folded_measurement::folded_bytes_bounded` for the same reason: the
recursive stat-only fold is a folded *measurement*, and
`folded_measurement.rs` is the one module on this path allowed to
traverse. Adapters reach it through `IdentifyCtx::folded_bytes`.

Since 2026-09-22 the audit also follows the *callee*, not only the
allow-list. The independent re-review's sharpest audit finding was that
"where a re-walk is written" is not "whether a re-walk happens":
`folded_measurement.rs` was on the allow-list and
`folded_measurement::measure` called `walk::resize_artifact_excluding`
unconditionally, fully re-walking every external root on every pass,
while this audit stayed green and the statement above claimed the
opposite. The audit now requires `measure` to consult the persisted
folded rows (`reuse_folded_measurement`) *before* it may reach
`resize_artifact*`, so the statement and the code cannot diverge again.

**Limits.** This is a syscall-shape check plus one call-order check
inside `measure`. It cannot prove the reuse is *correct* — that an
unchanged root really is unchanged; the work-counter tests below
measure that, and the reuse's own blind spot is stated on
`folded_measurement::reuse_folded_measurement`: the stamp is each
directory's `mtime`/`ctime`, so a file rewritten **in place** (same
name, same directory) does not move it. If such a rewrite also changes
the file's allocation, the reused byte total is stale until something
else in that directory changes. The alternative is stat'ing every file
on every pass, which is the "scales with all files" shape the handoff
forbids.

Since 2026-09-22 the **agent** family has the same reuse, keyed per
container directory (`crate::agents::ContainerCache`,
`IdentifyCtx::container`), and therefore the same blind spot — with one
consequence that is materially larger than it is for external caches
and is recorded here rather than left for the next reviewer to find:

> A tool **appends to an open session transcript in place**. An append
> moves the file's own `mtime` and size but not its parent directory's
> `mtime`/`ctime`, so a container whose shape has not changed is
> replayed and the growing session's stored byte total stands. The
> container is re-identified the moment anything is created, removed or
> renamed inside it — which for a session home is the next new session,
> the next companion directory, the next `todos/` entry — and the real
> size is reported then. Reporting lag, bounded by the container's next
> shape change; never an authorization hole, because every execution
> sink re-derives from the live filesystem with both caches disabled
> (`reidentify_for_tool`).

Two runtime tests pin this rather than leaving it as prose:
`a_session_rewritten_in_place_is_not_seen_until_its_container_moves`
(bytes) and
`relinking_a_session_to_a_different_project_leaves_bytes_and_growth_history_unchanged`
(declared linkage, plus the fresh re-identification that does see it).

The cost of removing the lag is exactly one `stat` per session file per
pass, which is the shape the handoff forbids; the cost of keeping it is
a stale byte total for actively-appended sessions between container
shape changes. That trade is stated, measured and flagged for
adjudication — it was not chosen quietly.

## Runtime tests that complete it

- `crates/core/tests/incremental_external_and_agent_measurement.rs` —
  work counters (`crate::work_counters`) over a synthetic 5,000-session
  agent home and a 20k-file external cache root: first observation reads
  headers; an unchanged second observation reads **zero** header bytes
  (asserted as `== 0` since the identification cache landed); since
  2026-09-22 it also lists at most one directory per *container* and
  pays at most ten stats per container over 5,000 sessions (measured:
  11 listings / 5,031 stats on the first pass, **5 listings / 31 stats**
  on the unchanged second, over 5 containers), and
  `appending_one_session_re_identifies_exactly_one_container` asserts
  that one appended session re-identifies exactly one container and
  replays the other four; appending one session costs
  at most one capped header read and exactly one cache miss; and, since
  2026-09-22, an unchanged external cache root lists **zero**
  directories and pays fewer than 100 stats over 20,000 files
  (`an_unchanged_external_cache_root_is_not_re_traversed`: measured
  6 listings / 20,004 stats / 73 ms on the first pass, 0 listings /
  4 stats / 1.9 ms on the second). The stat count is the unit's
  directory count, which is the claim: work scales with containers,
  not with files. `a_changed_external_cache_root_is_measured_again`
  is the other half — a file added under the unit costs a real
  re-measurement and its bytes are reported.
- `crates/core/tests/reviewer_cost_measurement_stack2.rs` — two
  unchanged full observations over a multi-ecosystem fixture, measured
  with an instrumented `walk.rs` rather than an instrument blind to it.
  Zero header bytes and zero subprocess spawns hold. Its `dirs_listed
  == 0` / `files_statted == 0` assertions **do not** hold. The exact
  residuals on the unchanged pass, measured 2026-09-22 after the
  container-level reuse landed, with the fixture's parts observed
  separately so the number is attributed rather than assumed:

  | part of the unchanged pass | `dirs_listed` | `files_statted` |
  |---|---|---|
  | the walk alone (`ObservationParts::WALK_ONLY`) | 34 | 5,553 |
  | the agent family's share | 6 | 33 |
  | the external family's share | 0 | 19 |
  | **total (`ALL`, what the reviewer test asserts on)** | **40** | **5,605** |

  The agent family's share fell from 12 listings / 5,027 stats to 6 / 33
  over 5,000 sessions. The **walk** is now the whole of the residual,
  it is identical on both passes, and it is not reachable by the reuse
  in this fixture: pass 1 takes the `full_rules_changed` branch of
  `growth::observe`, which deliberately anchors no FSEvents id (there
  was no replay), so pass 2 replays from nothing and refuses with
  `no_stored_event_id`. Anchoring an id at the start of a full walk
  would make the second pass depend on `fseventsd`'s own log lag — the
  reason `RefreshRefusal::TooSoon` exists — so it is timing-dependent,
  not deterministic; and the reviewer test constructs its own source
  (`fs_events::platform_source()`), so no fixture source can be
  injected without editing a file this chunk may not edit.

  Even a zero-cost walk would leave `files_statted` at 52 — the
  container and folded-row stamp checks, which are the reuse's *own*
  cost and are counted as the real `stat`s they are. `== 0` is
  therefore not reachable without reclassifying counted work, which is
  the vacuous-instrument sin the re-review named. Left failing, with
  the numbers, for re-review 3 to adjudicate.
