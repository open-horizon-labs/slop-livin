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

## The fix (second cut; the first was rejected)
The first cut (`26fcfe2`) read the 8 KiB prefix through `ctx.derived`
and ran a tokenizer over it that decoded only `type` and the `cwd`.
Rejected by the owner, rightly: the instructions text begins ~600 B in,
so ~7.5 KB of it was in memory before the tokenizer saw a byte. The
packet's rule is about what is *read*, not what is kept.

Second cut: a new gate primitive `fs_gate::read::bounded_scan_header`
reads one byte per `read(2)` and stops at the byte the scanner marks
done, or at the cap (still a `BoundedCap`, still `header_at_most`);
it retains nothing and counts exactly the bytes fetched. Surfaced as
`bounded_io::scan_header` and `IdentifyCtx::derived_scanned`, which
memoises only the extracted value. The Codex adapter's `SessionMetaCwd`
is a byte-fed state machine: it tracks the key path, decodes key names,
the top-level `type`, and the `cwd` at `cwd` / `meta.cwd` /
`payload.cwd` / `payload.meta.cwd`, and returns Done at the `cwd`'s
closing quote. A record of another kind is refused at its `type`'s
closing quote, before any `cwd`. A `type` written after the `cwd` is
not consulted (reaching it would mean reading past the field); Codex
writes `type` before `payload` in 100/100 probed records, and the
older, `type`-less envelope shapes still resolve.

Tests: `the_read_ends_at_the_closing_quote_of_the_cwd_and_the_canary_
after_it_is_never_fetched` -- the canary begins one byte after the
`cwd`; `counters.header_bytes_read == head.len()` exactly, where `head`
ends with the closing quote; `the_scanner_stops_at_the_field_and_
refuses_at_the_record_kind` -- exact stop bytes for: cwd beyond the
ceiling (stops at the cap, `None`); cwd cut by the ceiling (`None`);
`turn_context` (refused at the type's closing quote); cwd in an array;
escaped path (stops at its closing quote, the `SECRET` after it unseen);
`payload.meta.cwd`; no `type` before the cwd; a non-ASCII path.
`bounded_io` has its own test of the primitive's stop and cap; the gate
compile-fail cases (`content_reads_need_a_cap`, `caps_are_named_
constants`, `no_unbounded_read_in_the_gate`) still pass with the new
primitive in the gate.

The source audit `adapters_do_not_reach_gates` rejected the first draft
of the scanner for an `.expect()` (a panic would print its payload);
replaced with a non-panicking path. That audit doing its job is worth
recording.

## Measurement (fresh mktemp store, same config as the folder-inference run)
| Codex | before (`139a1f6`) | after |
| --- | --- | --- |
| sessions linked/declared | 0 | **1,217 (590.6 MB)** |
| sessions missing (declared cwd no longer exists: deleted checkouts, temp worktrees) | 0 | 1,760 (874.2 MB) |
| sessions not-a-project (cwd exists, not a git checkout: e.g. a home dir) | 0 | 473 (42.5 MB) |
| sessions unresolved | 3,450 (1,487.5 MB) | **0** |
| archived-sessions | 23 unresolved | 23 missing |
| cold observe | 11.2-12.8 s | 11.5 s (prefix parse), 11.7 / 12.2 s (byte stream: ~1.3 M one-byte reads, within noise) |
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
- Claude Code's `read_header_cwd` still reads up to 8 KiB of the first
  records looking for the first one with a `cwd`; when that record is a
  user message, prompt text is in that buffer. Pre-existing, outside
  this task's scope, and the same rule applies: it should stream and
  stop at the field. Recorded, not fixed here.
- Records where `type` precedes `payload` and both fit in 8 KiB are the
  only shape verified locally; older Codex versions' first lines are
  covered by the pre-existing whole-line tolerance tests, not by
  observation.
- Archived sessions share the parser and the fix, but were not
  separately probed (23 units locally).
