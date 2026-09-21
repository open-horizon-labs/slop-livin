# Build decisions in the project tree

## Aim

Make build storage understandable and reviewable from the ordinary project tree,
with readable columns and actionable removal trade-offs. Preserve folded storage,
existing accounting/history and exact approved cleanup. No real cleanup in this task.

## Solution Space

Three options considered for each issue:

| Issue | Band-aid | Local improvement | Selected reframe |
|---|---|---|---|
| Formatting | More literal spaces | Fix badge width alone | Cell-budgeted columns, headings, less chart chrome, selected details |
| Navigation | Document key 3 | Default to Builds | Project-context build hierarchy with in-place expansion |
| Decisions | Repeat unchecked | Age/newest-hash ranking | Role-based removal trade-offs separate from authorization/disuse |

Decision criteria: findability without another view, readability at 80/120/200
columns, precise selection, truthful advice, no new scan/persistence cost.
Current-frame approaches leave discoverability or usefulness unresolved. The
selected approach tests whether project-context consequences support a decision;
it does not claim to solve superseded-variant attribution with absent evidence.

Lineage: BA0 under W0, contributing to BA1 presentation (#72), BA2 exact group
selection, BA3 validation. BA1b history is preserved unchanged. These corrections
do not complete GBA or the broader cross-ecosystem epic. Owner: this task;
review trigger: hidden consequences, widened action scope, or costly rendering.

## Execute

Implemented grapheme/cell-aware truncation, separated badges, column labels,
responsive bar/fact visibility, selected details and scrolling. Cargo categories
appear directly in the project tree. Groups are reached through those categories;
category navigation keys are separate from actionable unit identities. A transient
parent index uses existing folded rows, with no disk walk or persistent index.

Shared derived guidance explains compiler-cache rebuild cost, executable recovery,
build-script consequences and unsupported dependency cleanup. Recommendations
neither authorize actions nor infer last use or supersession from timestamps.

## Risk retirement plan

- Cell sizing: widths 0–79 with CJK, combining marks, emoji sequences and 12-worktree
  badges must defeat character-count padding. Render frames at supported widths.
- Navigation: opening a project must expose profiles/categories without Builds;
  category must not acquire a cleanup identity; leaf must retain its exact path.
- Long lists: selecting group 39 must scroll into an 80x24 frame.
- Advice: changing mtime cannot alter recommendation; incomplete coverage must
  suppress cache advice. This defeats old-means-obsolete and cosmetic unchecked copy.
- Existing locks/approval/hardlink checks remain exercised by the workspace suite.
- Accepted with rationale: future usefulness, unavailable toolchains and exact
  freed bytes need human judgment. Reliable superseded-build classification still
  needs variant/consumer evidence; no such detection is claimed here.

## Review

Review uses the code lens and the existing BA guardrails. Initial tests exposed
long labels hiding category status and fixture folding before drilldown; corrected.
Mechanical layout detector reports no findings (Rust coverage is limited).
Human verification remains: real terminal/font rendering and whether the advice
supports the user's retention decision. Verification results follow below.

### Verification

- Full workspace tests passed, including existing action/authorization, hardlink,
  history and incremental tests; two pre-existing ignored tests remain ignored.
- TUI: 49 unit tests and 19 integration/frame tests passed. New tree test covers
  a small folded target, category/leaf distinction, 40 groups and scrolling.
- New recommendation test passed: timestamp changes do not become disuse evidence;
  incomplete coverage suppresses recommendations and remains blocked.
- Strict workspace Clippy, formatting, diff whitespace and all 19 source audits
  passed. Regenerated and inspected 80/200-column frames; 120-column behavior is
  asserted by the new tree test. Unicode widths 0–79 are bounded by tests.
- Initial full-suite failure was test-environment contamination from an exported
  CARGO_TARGET_DIR; rerunning with Cargo's --target-dir option passed without
  altering config-discovery behavior.
- Preserved source checkout's staged planning documents and Homebrew installation.
  No user data removed, no plans authorized, no release/install performed.

Review verdict: layout/navigation corrections and consequence-based guidance are
implemented. Reliable superseded-variant detection remains absent, not completed
by relabeling rows. This delivery contributes to BA1/BA2/BA3, not full epic closure.

## Aim correction: aid cleanup, not perfect advice

User explicitly clarified that merely old is sufficient evidence for a cleanup
suggestion. The prior framing over-weighted proof of supersession. Now supported
groups receive a cleanup-candidate recommendation with measured modification age.
Both in-tree groups and CLI candidate pages rank oldest-known modification first,
then size, with unknown/future timestamps last. No minimum age or access-time
dependency, no additional filesystem walk, no automatic deletion. Recent groups
remain reviewable. Role-specific rebuilding consequences stay alongside advice.

Tests distinguish ranking from eligibility: a 21-day-old smaller group outranks a
larger recent group; the recent group still qualifies; zero/future times are not
treated as ancient; incomplete coverage keeps existing action checks. This does
not claim last execution or certainty of obsolescence. That uncertainty is an
accepted advisory trade-off, not a blocker for useful recommendations.

## Candidate visibility / available-space iteration

Compared three presentations: prose in the facts column (observed failure),
separate age/status columns (too much fixed-column overhead at 80 columns), and
compact category summaries plus selected-category previews (selected). Initial
rendered summary variant still spent too much width padding paths; capped names
at 64 cells and rendered candidate evidence in the unused lower area.

Collapsed rows now show count, allocated candidate bytes and oldest modification;
individual rows show candidate/manual status and age. Zero-byte and unsupported
groups are excluded from opportunity summaries. Summary traversal stops at each
selectable group, avoiding parent/descendant double counting. Stars and a visible
legend distinguish nested allocation from report totals. Advice never changes
selection/approval rules.

Verification: 49 TUI unit tests, 19 frame/integration tests and strict workspace
Clippy passed. Batched frames at 80 and 200 columns checked; integration asserts
120-column behavior too. New assertions prove empty/unsupported groups do not
inflate the count and spare space contains an oldest-candidate preview. Actual
terminal/font review remains with the user; no cleanup performed.

## Purpose-based cleanup trial — 2026-09-21

User selected grouping by what removal gives up, not Cargo paths. Each profile
now leads with Compiler caches, Compiled tests & examples (expandable Tests and
Examples), and Build-script output. Physical layout remains under Inspect
directories. These are transient report projections, not stored identities or
new filesystem scans. Existing exact-unit checks and confirmation remain.

Space resolves a virtual group to nonempty supported exact members, excluding
unrelated dependencies, duplicate paths and nested selected directories. All
marked toggles off; partial selection adds remaining members. A failed member
check rolls back newly added marks, preserving previous selections. No virtual
group can fall back to deleting its containing directory. Spare-space previews
show combined members and consequences.

Verification: 49 TUI unit tests and 19 integration/frame tests passed (fixture
Git signing disabled only in the test process); strict TUI Clippy passed. Added
checks cover combined test/example sizes excluding dependencies, exact cache
selection/toggle, rollback after a later missing member, and unsupported/empty
members. Frames checked at 80 and 200 columns. No real-user cleanup performed.
The user will judge whether the grouping is clearer in the installed preview.

## Responsive cleanup and cancellation — 2026-09-21

The real 617-group cleanup took 82 seconds with synchronous execution on the UI
thread. All groups completed in the ledger, but the screen and keyboard froze.
Raw-mode Ctrl-C was also not handled. Grouping made an existing synchronous
action path much more costly; fixture correctness alone missed usability.

UI marking/review and confirmed execution now run on worker threads with progress
messages. The main loop renders a Deleting group-count gauge, elapsed seconds,
success/refusal counts and current path. Review has its own progress state. No
second action or report replacement occurs while busy; pre-delete observation
results are discarded. Human authorization is still created on confirmation,
and existing overlap, identity, lock, occupancy and ledger checks remain intact.

Esc/Ctrl-C/q while busy requests cancellation between groups, allowing the current
group to finish and record its outcome. Idle Ctrl-C exits. Completed deletions
are not undone; refused and unattempted marks remain. Cancelled review preserves
the prior selection, including when a result was already queued. A disconnected
worker reports failure rather than leaving a permanent busy screen. Terminal
errors request cancellation and wait for the current operation boundary.

Verification: 54 unit tests and 20 integration/frame tests, strict TUI Clippy and
19 source audits passed before final preview rebuild. Fixture tests cover
background completion, cancellation at a group boundary with ledger evidence,
preserved marks, worker disconnect, Ctrl-C busy/idle semantics, and progress at
80/200 columns. No real-user cleanup was run to test this change. Cancellation
is deliberately not an interrupt inside a filesystem move or current check;
the Cancelling state says so. Actual terminal responsiveness remains a user
verification point when trying the installed preview.

## Profile selection — 2026-09-21

Profile rows previously had no action mapping and returned nothing to delete.
They now resolve to supported exact descendants through the same selection path
as purpose groups; the label states Review supported groups only. This is not
whole-profile deletion: dependencies/final outputs remain outside that selection.
The regression test dispatches Backspace through the real background key-handler
path for both Compiler caches and profile debug, waits for review, and checks
that only the expected exact member is marked, never the profile directory.
The reported Compiler caches refusal was not reproduced in the current build.
The user's running process predated the latest installation; restart is required.
74 TUI tests and strict TUI Clippy passed. No user data removed during testing.
# Release review — 2026-09-21

Aim: make build cleanup understandable and selectable in the normal project tree without freezing interaction or overstating recoverability/reclaimed bytes.

Review: continue to ship as v0.6.3. The full workspace tests passed, including 54 TUI unit tests, 20 frame/integration tests and the four AST guard tests. Strict workspace/all-target Clippy, formatting and all 20 source audits passed. The version bump changes no behavior; release CI repeats the workspace tests in the optimized shipping profile.

Risk retirement: profile/purpose selection resolves exact supported descendants, never the whole profile; regression fixtures assert selection. Cancellation preserves previous review selections and durable completed moves; refused/unattempted selections remain marked. Worker disconnects surface errors. Progress and narrow/wide frames are tested. The AST guard rejects known blocking action paths but is not a type-resolved or exhaustive nonblocking proof. Allocated/hardlinked bytes remain explicitly non-additive, not promised free space. No user data was deleted during release validation.

Human verification: the user has reviewed the real grouped tree screenshot and found it useful. Automated fixtures verify progress/cancellation; future terminal/backend performance remains observable behavior, not a universal latency guarantee. Full cross-ecosystem coverage and perfect obsolete-build detection are not claims of this release.

Plan reconciliation: existing #64–#66 foundations are partially/substantially implemented, not blank-slate work. Their acceptance-level gaps remain open. #72/#100 now allow per-adapter delivery; full epic scope remains required. Next: standalone #103/#104, then project-linked agent storage, coverage/evidence alongside it, Linux independently, and Node-first expansion of build adapters.
