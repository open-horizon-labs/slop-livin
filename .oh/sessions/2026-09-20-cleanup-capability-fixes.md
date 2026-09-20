# Cleanup capability fixes

## Aim

Let a developer or agent find and propose meaningful Cargo groups and worktrees
without parent-agent workarounds. Uncertain reclamation is an accounting limit,
not a reason to prohibit moving explicitly selected hardlinked files to Trash.

## Constraints

- Preserve folded directory observations, Parquet reverse deltas and incremental
  event-driven walks. No background per-file inventory or global inode index.
- Review only selected groups. No automatic widening to a category or parent.
- Human authorization remains required. Real-repository trials may inspect and
  create unapproved plans, never approve or execute them.
- Selected allocation, recoverable Trash contents and freed disk space differ.
  Do not claim age, a hashed name or a passing check proves disuse.

## Implementation and review iteration

Two Luna workers implement root-scoped observation persistence and hardlink
cleanup independently. Parent integrates candidate coverage/paging and visible
accounting warnings. An independent Luna reviews the CLI workflow.

First-pass review required adjustments:

1. Hash-key tests alone do not prove the failed root-switch workflow. Require
   report, no-observe, proposal, history and canonical-alias integration tests.
2. A canonical namespace with caller-spelled topology still mixes path forms.
   Normalize report/cache paths as well as namespace identity.
3. Hardlink counts must not become a new estimate-based refusal. Keep selected
   membership, identity and contents checks independent of allocation/link counts.
4. No-hardlink allocation is not exact reclaimable space (clones and snapshots
   exist), and same-filesystem Trash moves do not immediately free those bytes.
5. A bounded page is not the whole opportunity. Expose all observed candidates
   in scope, allocation totals, unchecked count and next-page arguments.
6. Independent review found that noncandidate coverage could vanish and that
   `--within` included the directory it promised not to select. Separate coverage
   reporting from candidate eligibility; keep candidate selection strictly below
   `--within` and use `--path` for the exact group.

Root namespaces deliberately do not import the old ambiguous device-only state.
Old files are untouched; a root's new namespace starts its own baseline. This is
not a migration project, and it must not invent a trustworthy root attribution
for old mixed-root observations.

## Trials

First uncoached Luna trial used one temporary store, the release-built candidate,
CLI help and output only. Parent observation: 7.41 s for 23 projects / 63.5 GB
walked. A five-group incremental check took 1.84 s and created unapproved plans
for 1,118,973,952 allocated bytes; 857 of 862 candidates remained unchecked.
Hardlinks no longer blocked those plans. No real files were moved or approved.

This was **not** accepted as full task completion: the agent prioritized the
active implementation worktree and did not finish the main-repo/worktree
comparison. A follow-up asked it to finish that outcome without supplying
commands or workarounds. Successful mechanics alone are insufficient evidence
that the recommendation serves the user's aim.

Follow-up main-repo review found 535 incremental candidates totaling
9,834,860,544 allocated bytes. Five selected groups totaling 918,728,704 bytes
produced unapproved plans in 2.82 s; 530 remained unchecked. The clean linked
issue-105 worktree carried merged PR #106, reachable-tip and zero-unpushed facts.
The agent still chose a group inside it rather than a whole-worktree proposal.
Inspection found that `propose --path` help incorrectly described only artifact
paths. Corrected the help and usage examples to expose existing worktree support;
a fresh whole-worktree trial is the remaining usability check.

That fresh trial passed using only the new CLI's help/output and the same store:
it selected `/Users/muness1/src/open-horizon-labs/swamp-issue-105-ecosystem-salvage`
as a whole linked worktree, not a group inside it. Proposal
`294de406-2000-4810-868a-0cfbbe9e287c` has verb `remove-worktree`, measured size
3,920,982,016 bytes, and clean / zero-unpushed / merged PR #106 / reachable-tip
evidence. It remained unapproved. The other clean worktree's unknown upstream
and merge status were retained as uncertainty. Proposal took 1.64 s.

Final trial polish: include an exact worktree proposal example in help and a
measured-size line in `report --worktree`, avoiding a second drill-down just to
obtain size. Clarify that retained measurement `observed_at` differs from plan
`created_at`; do not fabricate freshness by restamping reused measurements.

Parent verification of the updated release-built CLI: main-repo `report --json
--no-observe` took 0.20 s, returned 909 Cargo rows, and reported incremental replay
with two changed directories. This is a single machine/run, not a latency SLA.

## Dissent

Steel-man: isolate roots using the existing store layout, permit same-filesystem
hardlink moves through the existing authorization boundary, and make bounded
review coverage explicit. This addresses observed failures without a new index.

Pre-mortem:

- Functional: alias paths or a narrower observation poison later proposals.
  Test actual alternating-root workflows, not just distinct hashes.
- Adoption: the agent still reports a tiny first page as all reclaimable space,
  or needs a parent to supply hidden commands. Require an uncoached CLI trial.
- Opportunity cost: solving accounting perfectly adds another recursive inventory
  and delays useful cleanup. Keep unknown reclamation explicit; inspect only the
  selected groups when asked.

The weakest assumption is that discoverable commands plus passing group checks
are enough to support the real task. Fixture tests cannot establish this alone.
Cross-review found no remaining core blockers. Eight hardlink fixtures cover
retained external aliases, internal aliases, link-only changes, changed contents,
changed membership, authorization and Trash-versus-free accounting. Ten
incremental/history tests include actual alternating-root report/proposal use.
Current recommendation: **Proceed**. The whole-worktree trial passed, and the
remaining reported help/size friction was corrected without changing selection
or authorization semantics.

## Review

Status: **Continue**. The frame remains
developer storage management, not filesystem auditing. No expanded deletion
authority is implied. The full workspace suite passed (263 tests, two ignored),
all 19 source audits passed, release build passed, formatting and diff checks
passed. Independent review findings were fixed and cross-review found no core
blockers. No standing grant support was added for Cargo groups.

The standard check script exposed pre-existing Clippy failures in Cargo code.
A final Luna worker fixed the mechanical nested-if/borrowed-slice/allocation
warnings. Two existing high-arity helper functions received narrowly scoped,
documented lint allowances instead of an unrelated refactor. Strict workspace
Clippy now passes; no global suppressions or behavioral model changes were used.
Final `scripts/check.sh` passed in full: formatting, all workspace tests, strict
Clippy, all 19 AST audits and destructive/verdict source checks. The final
release-profile CLI build also passed. An audit caught the new size label's
word "stale"; it now says the unique-byte estimate needs reconciliation.

Risk retirement: roots/history/aliases exercised through report and proposal;
estimate-only refusals removed and adversarially tested; selected content and
authorization checks retained; folded-storage/incremental invariants pass the
existing tests and AST audits. No per-file inventory or new database was added.

Human judgment remains necessary to decide which builds are worth retaining and
whether a worktree's local work is disposable. Tests and agents cannot establish
future usefulness or authorize the user's cleanup on their behalf.

## Delivery

Changes are local on `cleanup-capability-fixes`, with an unreleased CLI build in
`target/release/swamp`. No release, install replacement, real cleanup, or real
authorization was performed. All trial plans remain unapproved and may expire;
propose again before any later human-authorized cleanup.
