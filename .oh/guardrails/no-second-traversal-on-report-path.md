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
unchanged root really is unchanged; the work-counter tests below prove
that.

## Runtime tests that complete it

- `crates/core/tests/incremental_external_and_agent_measurement.rs` —
  work counters (`crate::work_counters`) over a synthetic 5,000-session
  agent home and a 20k-file external cache root: first observation reads
  headers; an unchanged second observation reads **zero** header bytes
  (asserted as `== 0` since the identification cache landed, with
  `identification_cache_hits >= SESSIONS`); appending one session costs
  at most one capped header read and exactly one cache miss; and, since
  2026-09-22, an unchanged external cache root lists **zero**
  directories and stats **zero** files
  (`an_unchanged_external_cache_root_is_not_re_traversed`, no longer a
  measured known gap).
- `crates/core/tests/reviewer_cost_measurement_stack2.rs` — two
  unchanged full observations over a multi-ecosystem fixture: zero
  header bytes, zero directories listed, zero files statted, zero
  subprocess spawns, measured with an instrumented `walk.rs` rather than
  an instrument that is blind to it.
