# Codex sessions link again: bounded early `cwd` extraction (2026-09-25)

Branch `claude/agent-project-attribution-hybrid`, after
`2026-09-25-agent-folder-inference.md`. Scope: the Codex adapter's
header parse, its tests, and the docs that had claimed Codex carries
no `cwd`.

## The defect
`agents::codex::read_header_cwd` read the bounded 8 KiB prefix of a
rollout and then required the *whole first line* to parse as JSON. On
the owner's machine that line never parsed, so no Codex session had a
`cwd`: 3,450 units, 1.49 GB, all `unresolved`. The earlier spike note
("no cwd in any record within the bound") described the parser's
result, not the file.

## Structural evidence (100 most recent local rollouts; key names and
## byte offsets only -- no values read into any output, nothing retained)
| fact | count |
| --- | --- |
| first record longer than 8 KiB | 100/100 (median 22,457 B, max 48,460 B) |
| first record `"type":"session_meta"` within 8 KiB | 100/100 |
| `payload.cwd` key within 8 KiB | 100/100, at 220-334 B |
| what makes the record long | `payload.base_instructions.text` (the project's instructions file) begins ~600 B in |
| `payload.git` key present in the first record | 92/100, at 18,546-48,291 B -- never within the bound |
| `payload.forked_from_id` within 8 KiB | 9/100 |
| a later `turn_context` record carrying `cwd` within the first 64 KiB | 9/100 |
| `payload.runtime_workspace_roots` | present (an array; one string the same length as `cwd` in the probed record) |

Observed first-record key order: `timestamp`, `ordinal`, `type`,
`payload.{session_id, id, forked_from_id?, forked_from_ordinal_exclusive?,
timestamp, cwd, runtime_workspace_roots, originator, cli_version,
source, thread_source, model_provider, base_instructions.text, ..., git?}`.

## The fix
`session_meta_cwd_from_prefix`: a tokenizer over the prefix that tracks
the key path, decodes exactly two strings -- top-level `type` and the
`cwd` at `payload.cwd` or `payload.meta.cwd` -- and skips every other
token undecoded. A string the bound cuts through ends the scan; if that
string is the `cwd`, the answer is `None` (never a prefix of a path).
`type != session_meta` -> `None`. The read bound is unchanged; the
whole-line parse still runs first for records that fit (older shapes).

Tests (`agents::codex::tests`): `a_first_record_longer_than_the_bound_
still_links_by_its_early_cwd` (3x-bound instructions text with a canary
after the `cwd`, both inside and beyond the bound: Linked/Declared,
one capped read, `no_content_leak`), `the_prefix_extractor_takes_one_
supported_field_and_nothing_else` (cwd after the bound; cwd cut by the
bound; `turn_context` record; untyped record; cwd inside an array;
escaped path; `payload.meta.cwd`; `type` after `payload`).

## Measurement (fresh mktemp store, same config as the folder-inference run)
| Codex | before (`139a1f6`) | after |
| --- | --- | --- |
| sessions linked/declared | 0 | **1,217 (590.6 MB)** |
| sessions missing (declared cwd no longer exists: deleted checkouts, temp worktrees) | 0 | 1,760 (874.2 MB) |
| sessions not-a-project (cwd exists, not a git checkout: e.g. a home dir) | 0 | 473 (42.5 MB) |
| sessions unresolved | 3,450 (1,487.5 MB) | **0** |
| archived-sessions | 23 unresolved | 23 missing |
| cold observe | 11.2-12.8 s | 11.5 s |
| longest string field in any Codex unit's JSON | -- | 153 chars; no instruction text |

Claude Code sessions in the same run: 29 declared / 119 inferred /
3 unresolved (one more than the earlier run: a new session under a
path outside the known worktree set).

## Corroboration spiked, not adopted
- `payload.git.{branch,commit_hash,repository_url}`: real corroboration
  of the `cwd` (and the only way to notice a moved checkout), but it
  sits after the instructions text; reaching it means raising the bound
  through 18-48 KB of a file that is, by the privacy contract, content.
  A future design could seek to a known offset only if Codex ever
  writes `git` before `base_instructions`; it does not today.
- `payload.forked_from_id`: names the parent *session*; useful to say
  "forked from <session>" (fork lineage), not to link a project. No
  SQLite lookup was made or is proposed.
- `turn_context.cwd` per turn: later lines; would need reading the
  transcript body. Out.
- `payload.runtime_workspace_roots`: within the bound and the shape of
  Oh My Pi's `additionalDirectories` -- a candidate for
  `project_link_declared_workspace` (widening to `Shared` when it names a
  second project). Not adopted without a second look at what Codex
  populates it with; recorded as the next cheap gain.

## Gaps
- Records where `type` precedes `payload` and both fit in 8 KiB are the
  only shape verified locally; older Codex versions' first lines are
  covered by the pre-existing whole-line tolerance tests, not by
  observation.
- Archived sessions share the parser and the fix, but were not
  separately probed (23 units locally).
