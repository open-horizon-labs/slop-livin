# Claude Code session -> project linkage by folder name (2026-09-25)

Branch `claude/agent-project-attribution-hybrid` off stack/26 @ 982627c,
isolated worktree. Scope: project-linkage identification, persistence /
replay, tests, narrow docs. No new walk; the known-worktree list is the
one `discover_and_measure_in` already receives.

## Design
- `LinkBasis::Declared` gains `folder_slug: Option<String>`; only the
  Claude adapter sets it (`AgentUnitBuilder::project_link_declared_or_folder`),
  from `projects/<slug>`'s directory name. The adapter never decodes or
  matches anything.
- `agents::KnownWorktrees` (slug -> distinct known paths) and
  `claude_folder_slug` (`[^A-Za-z0-9]` -> `-`, verified against a real
  header: `/Users/x/.codex/.chatgpt` -> `-Users-x--codex--chatgpt`).
- `ContainerCache::with_known_worktrees` + `finish_link`: the single
  resolution path for fresh and replayed units. Inference runs only when
  the declared resolution is `Unresolved`; `Missing`/`NotAProject` stand.
  Unique match -> `Linked { source: Inferred }`; two+ -> `Unresolved`
  ("re-encodes N known worktree paths"); one that is no longer a checkout
  -> `Unresolved` naming it; none -> the original reason.
- Persistence: the slug rides in the `link_declared` cell after a
  `\u{2}` separator; `CONTAINER_VERSION` bumped so older rows miss.
- Provenance: `LinkSource::Inferred` already existed in the model, serde
  and the Parquet unit columns; adding a field to `Linked` would have
  touched 56 sites, so provenance is the `source` field plus explicit
  text/TUI labels.

## Measurement (scratch SWAMP_DIR, ~/src + claude-code + codex only)
| | baseline 982627c | new |
| --- | --- | --- |
| Claude sessions linked/declared | 29 (300.5 MB) | 29 (300.5 MB) |
| Claude sessions linked/inferred | 0 | **119 (191.3 MB)** |
| Claude sessions unresolved | 121 (199.3 MB) | 2 (8.1 MB): "no cwd field in the session's first records" (slug matched no known worktree) |
| Claude missing / not-a-project | 8 (105.6) / 10 (319.5) | same |
| Codex sessions unresolved | 3,450 (1,487.5 MB) | same (no folder signal, no cwd in bounded header) |
| observe cold (3 samples each, fresh scratch stores) | 11.5 / 11.2 / 11.9 s | 12.8 / 12.0 / 11.8 s |
| observe replayed (unchanged second pass) | -- | 2.7 s |

## Risks
- Accepted: a session whose true path collides (lossy slug) with a
  different known path that is the only match is indistinguishable
  from a true match. Labelled inferred, recomputed each pass, never
  treated as declared. Documented in `docs/agent-storage.md`.
- Accepted: non-ASCII path characters are mapped per `char`; if Claude
  Code encodes per byte the slugs differ and no link is made (safe).
- Not done: path renames are not recovered; `Moved` is never inferred.
