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
