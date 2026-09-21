# Cleanup and recovery lifecycle

Swamp separates four steps, each a separate command, each independently
inspectable:

1. **Propose** (`swamp propose`) -- build a plan from report rows.
   Deletes nothing. Always returns `awaiting-authorization`.
2. **Approve** (`swamp approve <plan_id>`) -- a human writes a one-shot
   grant scoped to exactly that plan. **You never run this on the
   human's behalf** -- see `../SKILL.md` and `trust-model.md`.
3. **Execute** (`swamp execute <plan_id>`) -- re-derives every unit at
   the sink (still an artifact directory, no activity since the plan,
   not occupied) before acting. Per-unit outcomes name the fact behind
   any refusal.
4. **Recover** -- filesystem removals go to Trash with a restore
   manifest; Docker removals do not (see the table below).

## `swamp propose <root> [--filter F] [--path P ...] [--since S] [--json]`

```sh
swamp propose ~/src --filter 'kind:BuildOutput type:rust age > 30d'
swamp propose ~/src --path /absolute/path/to/a-worktree
```

For a worktree, first inspect `swamp report <root> --view worktrees
--json` for dirty/unpushed/merge evidence, then pass its exact reported
`path`. The scan root must cover that worktree.

Units carry `kind`, `bytes`, `growth_bytes`, `recovery` (`local_rebuild`
| `network_fetch` | `irrecoverable` | Docker-specific permanent
warnings), `signals`, `verb` (`delete` | `remove-worktree` | `archive`),
`track` (git tracking status when known), and `warnings` (dirty,
unpushed, untracked content, no remote, git store) -- present them to
the human verbatim; they are the facts a human weighs before approving,
not something swamp pre-judges. Docker build-cache entries cannot be
planned: Docker exposes no per-entry removal for build cache, only
`docker builder prune`, which acts on everything at once.

Plans expire (default 30 minutes) and are single-use; `created_at` is
the review time, not restamped on reuse.

## `swamp approve <plan_id>` (human only)

Prints every unit with its facts again, then writes a one-shot grant
scoped to that plan id and its expiry. This is the human's confirm
step. For repeated work, a human can instead create a bounded standing
grant:

```sh
swamp grant add 'kind:BuildOutput idle > 30d' --budget 5GB --expires 7d [--max-units N]
swamp grant list --json
swamp grant revoke <grant-id>
```

Standing-grant predicates are unit-level only (see `filters.md`);
budget and expiry are required so a grant can neither live forever nor
be unbounded.

## `swamp execute <plan_id> [--keep-executables] [--json]`

Refuses per unit with the specific fact, not a generic denial: no grant
covers it (`awaiting-authorization`), activity changed since the plan
was proposed, the grant's budget or unit cap is exhausted, the plan
expired, or the plan already executed. Nothing is touched when refused.

`--keep-executables` copies supported build outputs to `<worktree>/bin/`
before trashing the build directory (Rust `target/{release,debug}`
executables, Python `dist/*.whl` and `build/**/*.so`) -- it is not a
backup of everything in the directory, only those specific outputs.

| Unit | Removal and recovery |
|---|---|
| Filesystem path | Moved to Trash; swamp records the recovery location. Bytes remain on disk until Trash itself is emptied. |
| Linked worktree or checkout | Moved to Trash through its specific action path; inspect warnings about local work and repository context first. |
| Docker image | Removed by Docker. Recovery depends on the image still being pullable or reproducible -- no Trash. |
| Docker volume | Removed by Docker; swamp makes no copy of its contents -- irrecoverable. |
| Docker build-cache record | Reported, but individual removal is refused (see above). |

The ledger (`~/.local/share/swamp/ledger.jsonl`) records every executed
outcome with its actor string, grant id, and evidence -- independent of
the index, and never deleted by ordinary operation. Trashed bytes,
permanently-removed bytes, and measured free-space change are three
different numbers; do not conflate them.

## Reviewing Cargo build groups: `swamp cleanup-check`

A bounded, paginated review of Cargo build/incremental/test-executable
groups distinct from the general propose/execute flow:

```sh
swamp cleanup-check ~/src/my-project --role incremental --limit 5 --json
swamp cleanup-check ~/src/my-project --role incremental --limit 5 --offset 5 --json
swamp cleanup-check ~/src --within ~/src/my-project/target --role incremental --limit 5
swamp cleanup-check ~/src/my-project --path /absolute/path/to/target/debug/incremental/crate-group
```

Checks read group contents and create **unapproved** plans; they never
authorize or execute. Results distinguish `blocked`, `unchecked`, and
`ready_for_review` with reason codes. `next_page`/`next_command` in the
JSON are argument arrays, not shell strings -- keep the same
`SWAMP_DIR` across pages so plans it creates stay visible to
`swamp plans`. A five-group result, or a zero-candidate result, is
never a measure of the total cleanup opportunity -- see the
`coverage_limited_count`/`unknown_or_residual_count` fields in its JSON
output.
