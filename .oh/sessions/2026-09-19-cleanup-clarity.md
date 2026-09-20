# Cleanup clarity: execution, trials, review and shipping

## Aim and selected solution

Let a developer or agent distinguish storage categories from selectable Cargo
groups, review a bounded selection, and understand refusals without widening
scope. Preserve folded observations, existing hardlink/lock/freshness checks,
human authorization, and the distinction between allocation and reclaimed space.
Per-crate inspection stays in #107; hardlink cleanup support is separate work.

## Trial before implementation

Two independent agents used installed v0.6.0, without code access or instructions
about the right group level. Each had six command attempts / three minutes and
an isolated store. Neither could approve or execute cleanup.

- Main checkout: selected the incremental category, failed with "unsupported
  Cargo executable layout"; 912-line Rust output truncated. Report 1.681 s,
  builds report 1.387 s, failed proposal 0.089 s.
- Current worktree: same wrong category and refusal, 529-line output truncated.
  Report 0.683 s, failed proposal 0.723 s.

Both trials confused selection scope, not simply scan speed. Both correctly
avoided claiming old names established disuse or widening to the whole target.

## Execute

Added derived cleanup guidance shared by JSON and text/TUI: scope, status, reason,
next action. No per-file inventory or background eligibility walk. Rust text
output starts with accounting caveats and shows 30 rows unless --all is given.
Added cleanup-check: largest groups, optional role/exact paths, default five and
maximum twenty. It creates unapproved plans for successful reviews; failures do
not broaden the selection. Group count, not total bytes/time, is bounded.

Risk checks: category selected explicitly creates no plan; unknown paths refuse;
hardlinks still block; successful review remains unauthorized; exact companions
are retained; locks have a structured code and exact retry arguments; cache JSON
round-trips; report guidance does not inherit prior plan approval/readiness.
Unchanged existing 5,000-file folding/storage tests remain in the full suite.

## Dissent

The obvious copy-only patch would still require failed proposals. Conversely,
checking every group during refresh would damage incremental performance.
Derived guidance plus explicit bounded checks separates these costs. It does
not solve the actual hardlink cleanup coverage limit. A passing review is not
proof of disuse or a prediction of freed bytes. Readiness is not cached in rows.
No new cleanup authority was introduced; existing proposal/execution gates remain.

## Iteration and confirmation

Two fresh agents used the new CLI, without source inspection. Both distinguished
categories, unchecked groups and blocked results without category errors.
Main: five individual incremental groups correctly blocked for hardlinks, 4.608 s
across five CLI commands. Worktree: three hardlink refusals and three lock failures
while compilation was running; ~0.8–0.9 s per check command. Added lock_unavailable
and retry_after_builds plus next-command argument arrays in response.

Final main-checkout confirmation: cleanup-check --role test-executable --limit 3
created three proposed/unapproved exact executable+dep-info plans. 192,839,680
allocated bytes, not guaranteed savings. Check plus plan inspection took 4.192 s.
No approvals, execution or deletion occurred in any agent trial.

80x24 frame first exposed clipped status text. Moved concise category/unchecked
states into the signal column; the 80x24 and 200x60 confirmation frames preserve
those labels. Impeccable mechanical detector reported no findings on changed TUI
files. No unrelated design configuration was repaired.

## Review

Independent static code review found no blockers. Exact selection, no approval,
execution revalidation, bounded group count and JSON cache compatibility remain.
Human verification remains: do these states support confident decisions in daily
TUI use? Agent trials test discoverability, not human judgment about disuse.
Review is Continue for clarity delivery, not completion of hardlink cleanup.

## Ship

Planned v0.6.1: branch cleanup-clarity → PR/review → merge → version tag →
release-profile CI → public macOS arm64 archive → checksum and behavior checks →
installed CLI. No approval or cleanup of real artifacts belongs to this path.
