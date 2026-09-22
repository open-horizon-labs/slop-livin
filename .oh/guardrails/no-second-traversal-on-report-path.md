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
unchanged root really is unchanged. That is the job of the work-counter
tests below and of the gate the reuse now runs behind.

## The gate: trusted event coverage, not directory stamps (2026-09-22)

Both unit families originally decided "unchanged" from the recorded
directories' own `mtime`/`ctime`. A directory stamp moves when an entry
is created, deleted, renamed or replaced, and **not** when a file inside
it is appended to or rewritten in place.

For an external cache that is a corner case. For agent storage it is the
normal case: a tool **appends to an open session transcript in place**,
which moves the file's own size and mtime and not its parent's. A
stamp-keyed cache therefore reported a growing session at its old size
until the next create/delete/rename in its container. "What grew" is the
question this tool exists to answer, so the integration decision of
2026-09-22 removed stamp-only reuse as a sufficient condition for both
families, and replaced it with `fs_events::EventCoverage`.

A stored measurement or container may be replayed only when all three
hold:

1. some root this pass replayed successfully is the path or an ancestor
   of it — FSEvents was watching;
2. that replay reports no event at the path or under it — including
   writes into existing files, which is the part a stamp could not see;
3. the stored rows are no older than the observation the window opens
   from. A pass that skipped a unit family (`report::ObservationParts`)
   leaves a gap between when the rows were written and when the window
   opened, and no window can see into its own past.

With no window — a full walk, any `fs_events::RefreshRefusal`, an
overflow, a first observation, a store-less caller — there is no reuse.
The unit is re-measured or re-identified, which is slower and always
correct; the per-file identification cache still keeps header reads at
zero for files that did not move. Directory stamps are consulted by
neither family any more: they are strictly weaker evidence than the
window and cost one `stat` per recorded directory per pass, so keeping
them as a second opinion would buy nothing and charge for it. A replayed
container therefore costs **no syscall at all**.

**What this trades away.** The window comes from the walk, one per scan
root that went incremental. A tool home or external cache root outside
every scan root has no window and is re-measured on every pass, which is
the common shape of a default install (a scope of project directories
does not contain `~/.claude` or `~/.cargo`). Giving each unit root its
own FSEvents cursor is the obvious completion and is **not** implemented;
it is written up, with its ordering hazard, in
`.oh/sessions/2026-09-22-event-gated-reuse.md`. Until it lands, the
container/folded-row reuse is a real saving only where the scope already
covers the tool home, and the cost of that is measured below.

**What it never was.** None of this is an authorization boundary. Every
execution sink re-derives from the live filesystem with both caches
disabled (`agents::reidentify_for_tool`), and
`relinking_a_session_to_a_different_project_leaves_bytes_and_growth_history_unchanged`
asserts it directly — as well as asserting, since the gate landed, that
the *ordinary* pass sees a relinked session too, under event coverage
and under a full walk alike.

## Runtime tests that complete it

- `crates/core/tests/incremental_external_and_agent_measurement.rs` —
  work counters (`crate::work_counters`) over a synthetic 5,000-session
  agent home and a 20k-file external cache root.
  - First observation reads headers; a vouched-for second observation
    reads **zero** header bytes, replays all five containers, and
    re-identifies none.
  - `replaying_containers_costs_nothing_per_container` pins the
    per-container cost at exactly zero by holding the tool home's own
    structure constant and varying only the container count: nine
    containers cost the same listings and header bytes as one, and
    exactly eight more `stat`s — the eight entries `projects/` yields
    when the home is scanned, which is how the pass learns those
    containers exist. Twelve times as many sessions per container cost
    *nothing* extra.
  - `an_appended_session_is_seen_under_event_coverage_and_under_a_full_walk`
    is the decision itself: a 1 KiB append to an existing transcript is
    reported in the same pass under a window that names it, and in the
    same pass under no window at all. This test replaces
    `a_session_rewritten_in_place_is_not_seen_until_its_container_moves`,
    which asserted the old lag.
  - `appending_one_session_re_identifies_exactly_one_container` — one
    appended session re-identifies exactly one container and replays the
    other four, with exactly one derivation-cache miss.
  - `an_unchanged_external_cache_root_is_not_re_traversed` — a
    vouched-for external root costs **zero** listings and **zero**
    stats over 20,000 files, and `a_changed_external_cache_root_is_measured_again`
    is the other half.
- `crates/core/tests/agent_container_seams.rs` — the same two
  properties for every adapter whose session storage is a directory
  tree: Codex (`sessions/<yyyy>/<mm>/<dd>/`), OpenCode
  (`storage/session/<project-id>/`) and Pi (`sessions/<dir>/`). Plus
  `a_codex_day_container_does_not_depend_on_its_siblings`, which pins
  the property that made the conversion possible: each container owns
  its own entry budget, so its stored rows mean the same thing as a live
  identification of the same directory. A budget shared across
  containers — which Codex's session walk used to carry — did not.
  `oh_my_pi_declares_why_it_does_not_use_the_container_seam` records the
  one session tree deliberately left off the seam, and why.
- `crates/core/tests/reviewer_cost_measurement_stack2.rs` — two
  unchanged full observations over a multi-ecosystem fixture, measured
  with an instrumented `walk.rs` rather than an instrument blind to it.
  Zero header bytes and zero subprocess spawns hold. Its `dirs_listed
  == 0` / `files_statted == 0` assertions **do not** hold, and after the
  event gate they hold less well than before. Measured 2026-09-22:

  | unchanged second pass | `dirs_listed` | `files_statted` | header bytes |
  |---|---|---|---|
  | stack/12 (directory stamps) | 40 | 5,605 | 0 |
  | stack/13 (event gate) | 71 | 36,281 | 0 |

  The stack/13 numbers are **identical to that fixture's first pass**,
  which is the whole attribution: nothing was reused, because nothing
  could be. The fixture's one walked root is `src/`, an ordinary project
  directory; its Claude Code home, Cargo home, npm cache and model store
  are siblings of it, under no scan root at all. No replay window covers
  them, so condition 1 above fails for every unit and both families
  re-measure. A fresh two-pass fixture also has no FSEvents anchor for
  the walk itself — pass 1 takes the `full_rules_changed` branch of
  `growth::observe`, which deliberately anchors no id, so pass 2 refuses
  with `no_stored_event_id` — and the `RefreshRefusal::TooSoon` floor
  would refuse a back-to-back second pass in any case.

  This is the decision's cost, stated rather than smoothed: the gate
  bought correctness on the appended transcript and gave up the reuse
  everywhere a window does not reach. Both halves are the integration
  owner's call of 2026-09-22, implemented as written; the follow-up that
  would recover the cost without giving back the correctness (a per-unit
  -root FSEvents cursor) is named above and in the session note. Left
  failing, with the numbers, for re-review 3 to adjudicate.
