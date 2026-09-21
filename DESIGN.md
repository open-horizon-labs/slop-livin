# Terminal design

The UI presents disk growth as a table that opens into a project tree. Size, signed change, and activity facts stay close to the row they describe. Use [committed frames](crates/tui/tests/frames/) and the [renderer](crates/tui/src/ui.rs) to check the current behavior.

## Rows and hierarchy

Each row contains a name, bytes, signed growth, a change bar, and any visible facts. Project rows group checkouts and linked worktrees; tree rows show artifacts and the remaining directories. Box-drawing rails preserve parent-child relationships. Names truncate in the middle; numbers align on the right.

Column headings identify name, size, change, and cleanup/facts. Name truncation
and padding use grapheme-aware terminal-cell widths. Badges have separating
spaces. Build drilldowns reserve a compact candidates/oldest-modified column even
at 80 columns, hide the change bar, and hide numeric change below 100 columns.
Their name column caps at 64 cells. Other views hide bars below 140 columns and
show facts only in selected-row details below 100 columns. Zero and
unknown changes have no vertical bar. Keep the selected row visible when scrolling.

Collapsed build categories lead with a recommendation and removal consequence:
start with compiler caches (slower next build), review tests/examples (rebuild
before rerunning), and lower-priority build-script output (scripts rerun).
Counts, allocated candidate size, and oldest known modification age follow only
when space permits; narrow views drop these statistics before clipping advice.
Unknown age is `?`.
Candidate directory descendants are not counted again. Final outputs say
`Inspect only: removes built output`, without suggesting a supported selective action.
Other unsupported rows say `Selective cleanup unsupported`, not a safety verdict.
Nested allocated sizes have a `*` suffix and a persistent accounting legend.
When space remains below the tree, preview the selected category's oldest
candidates with paths, allocated sizes, ages, and rebuilding effects. This preview
is read-only: expand the category to select exact groups.

Opening a project shows Cargo profiles with purpose-based cleanup groups:
Compiler caches, Compiled tests & examples, and Build-script output. Groups
contain only present, nonempty supported members in that profile. Tests and
Examples are expandable subgroups. Space marks the exact members for review;
profile rows also select their supported descendants, never the whole profile
directory. Profile advice says Review supported groups only.
Marking a fully marked group clears its members. A failed member review rolls
back newly added marks, preserving earlier selections. No virtual group is a
directory deletion target. Age ordering applies within groups; individual members
can be selected instead of the whole group.

The physical tree remains under a collapsed Inspect directories row; it is a
second view of the same bytes, not additional storage. Physical category rows
are navigation, not selective cleanup units. The selected-row detail area shows recommendations
and rebuilding consequences. Compiler caches are a suggested starting point,
not a claim of obsolescence. No age-only or newest-hash-wins verdicts.

The change bar grows right for an increase and left for a decrease. Its length uses a logarithmic scale relative to visible changes. Changes below 1 MB use a small tick and dimmed text. The signed number supplies the actual value; the bar is not a linear scale of bytes.

Growth sorts descending by signed change. Other sorts cover size, name, ecosystem, and age. Tree traversal preserves hierarchy. Ecosystem glyphs follow project names; linked-worktree and build-output badges add context.

## Color

Growth is red, shrink is green, and secondary information is dim. The selected row uses a dark background (`Color::Indexed(236)`) and bold text. Marked rows use yellow and an `✗` prefix. Refusals use red text. Signed values and bar direction carry information independently of color.

The renderer uses terminal colors and an indexed selection color. Committed frames exercise 80×24 and 200×60 layouts; visual behavior on a particular terminal and palette still needs inspection.

## Navigation and filters

`→` opens or expands; `←` collapses or returns to projects. Enter opens a project or confirms an action. Esc cancels the active interaction or returns to projects. `/` opens the filter form; `:` edits the expression; `0` clears it. Parse errors retain the previous valid filter.

The initial filter is `growth > 100MB in 7d`. Saved filter and sort choices take precedence on later runs. Ten views are available through `v` and `1`–`9` (External is `9`; Agents has no dedicated digit -- `0` is "clear filter" -- and is reached only by cycling with `v`). The [usage guide](docs/usage.md#terminal-controls) holds the full key table.

## Header, progress, and history

The header shows the root, observation status, available history, and totals as space permits. It drops trailing clauses on narrow terminals. A cached report can appear while an observation runs in the background; the first run needs an observation before it can display data.

Observation progress shows walked bytes and directories. Its percentage is an estimate against the previous walked total. A live FSEvents watch batches changes after 400 ms of quiet. The header can display a history sparkline; body rows use change bars.

### External and Agents rows, and a scope-coverage header clause

`report_scope`/`external.rs` (#42/#43) and `agents.rs` (#91/#92) gave
the TUI three facts recorded here as unimplemented intent by an earlier
chunk; all three now ship:

- **External rows.** `ViewKind::External` (`'9'`) lists `ExternalUnit`s
  the same shape as `ViewKind::Unowned` lists unowned rows: path,
  category, size, growth, consumer count. The row is never markable
  (`Row.unit: None`) rather than markable-then-refused: the action
  layer (`actions::execute`) already refuses every
  `PlanUnit::external_category` unit unconditionally, so there is no
  delete affordance to offer in the first place.
- **Agents rows.** `ViewKind::Agents` (no dedicated digit -- `0` is
  "clear filter"; reached by cycling with `v`) lists `AgentUnit`s the
  same way: tool/category/relative-path/project-link facts. Every row
  carries `Row.unit: Some(...)` (protected/unsupported ones included):
  `Space`/`Backspace` mark the selected unit through
  `actions::propose_agents` (the same function `swamp propose-agents`
  uses) and open the confirm banner with its real consequences
  (session-removal loss warnings, the linked project); `Enter` executes
  through the ordinary background-worker path
  (`actions::execute_plan_progress`) every other markable view already
  uses -- never a new blocking call on the event/render thread. A
  protected/unsupported row cannot be marked: `propose_agents`'s own
  refusal (protected category, no supported action yet, active session)
  becomes the footer text, never a generic "nothing to delete." Bulk
  marking (`Shift+A`, `mark_all_in_view`) reaches agent rows too: since
  `model::agent_rows` sets `Row.unit` but never `Row.kind` (there is no
  `ArtifactKind` for an agent-storage unit), `mark_all_in_view` has a
  third branch alongside its `row.kind`/`ArtifactKind` and projects-view
  `row.project` ones -- when a row has neither but does carry `unit`, it
  reuses `mark_row`'s own per-row refusal (`propose_agents`'s protected/
  unsupported/active reasons) rather than duplicating that logic, and
  counts a skip instead of a hard stop. The footer names how many agent
  rows were skipped and why whenever at least one row *was* marked,
  never silently proceeding as if the skipped rows were not on screen.
- **Project tree's collapsed "Agent storage (linked)" row (#100
  completion).** `ViewKind::Tree`'s own drill (`model::tree_rows_with_agents`,
  built from the same `crate::tree::build_project_tree` the CLI's
  `--project` text drill uses) now appends one row per tool
  contributing linked agent storage to the selected project, after the
  project's own worktrees. It is informational only (`unit: None`):
  the row exists so "does this project have any linked agent storage,
  from which tools, how much" is visible from the project drill itself
  without also opening the separate Agents view -- acting on a specific
  unit still happens there, where per-unit protections/occupancy are
  checked. Absence of any linked unit means no row at all, never a
  zero-byte placeholder.
- **Scope-coverage header clause, now driven by real observation
  outcome (#51).** `App::set_scope_note` adds one short header clause
  when there is more than one region or the one region is not simply
  `Complete` (e.g. `2 roots (1 missing)`, `3 roots (1 inaccessible:
  permission denied)`, `2 roots (1 partial: 2 path(s) unreadable
  during this walk)`), dropped last by the existing "fit clauses to
  width" rule like every other trailing clause -- never silently
  hidden, just lower priority than size/growth facts on a narrow
  terminal. It now takes `&[coverage::RootCoverage]` -- this pass's
  *actual* per-root walk outcome (`RegionStatus`), the same shape
  `report_scope`/`report --json`'s `scope_coverage` field shows --
  instead of the pre-walk `scope::RootStatus` snapshot an earlier
  chunk shipped, which could not distinguish "part of a present root
  was unreadable during the walk itself" (`Partial`) from a clean
  `Complete` region. The common case (one `Complete` region) shows no
  clause at all, matching ordinary single-project usage.

### Multi-root reports, coverage inspection, and live refresh (#51)

`swamp ui` with no explicit root opens the TUI over the *whole*
configured scope, not just its first present root: `swamp_tui::run_scope`
calls `report::report_scope_with_parts` (the same coherent multi-root
entry point `report`/`observe` use) once at startup, giving the `App`
every present root's own report (`App::reports_by_root`) plus one merged
`Report` (`App::report`) built by folding them together
(`report::merge_reports`). This is what makes project/shared/external/
agent-tool storage from *any* included root show up together, including
a root with no Git checkout in it at all (previously invisible: the CLI
used to resolve the whole scope only to throw it away and hand the TUI
exactly one present root, which is all `swamp_tui::run` -- kept as-is
for the `swamp ui <explicit-root>` case -- has ever rendered).

Live refresh preserves this per-root separation instead of ever
replacing the whole merged report at once:

- `App::start_watch` opens one FSEvents stream per root in `App::roots`,
  all feeding one shared channel through cloned senders (`App::watches`
  is a `Vec`, not a single `Option<Watcher>`).
- `App::observe_live` handles one root's pending changes per call --
  whichever root owns the first pending changed path -- draining only
  that root's paths from `live_changes` and leaving any other root's
  changes queued for the next tick, so two roots going quiet in the
  same beat are never merged into one re-walk.
- `App::observe_in_background` (the cached-startup and post-delete
  refresh path) re-observes every root in `App::roots`, sequentially,
  in one worker thread.
- Both report their result(s) as `(root, Report)` pairs; `App::
  replace_report_for_root` updates exactly that root's entry in
  `reports_by_root` and rebuilds `report` from the *whole* map
  (`report::merge_reports`) -- a refresh of one root can never erase,
  stale-mark, or duplicate another root's rows, because that root's own
  cached entry is never touched.

Selection/filters/sort keep working unchanged: they operate on the one
merged `Report` exactly as a single-root report always has. A unit
shared across projects (an external unit's `consumers`, or agent
storage linked from more than one project's tree row) was already
counted once by `external.rs`/`agents.rs`'s own identity model (#43/
#91); multi-root scope does not change that -- merging per-root reports
only concatenates each root's own `projects`/`unowned` rows, it never
re-derives external/agent-unit identity.

Known, named simplification: `App::history_secs` (the growth-window
picker's bound) is still derived from the *primary* root (`App::root`,
`roots[0]`) only, not the narrowest history among every included root
-- a multi-root picker can currently offer a window longer than a
non-primary root's own store actually has. Revisit alongside #62's cost
validation.

## Actions

Space marks a row. Backspace opens the confirmation for the current row or marked set. Confirmation is a single inline row with selected paths, sizes, warnings, and destinations. Enter authorizes the action; Esc cancels it.

Project rows expand to actionable artifacts. If none exist, a direct project action may offer the checkout. Bulk marking with `A` skips that fallback. Worktree and source-directory selections carry their own warnings; the `ignored` and `untracked` summary buckets are not individual paths to delete.

Docker images and volumes must be named in the confirmation because their removal has no Trash recovery. Successful removals leave the displayed report, totals are adjusted, and the UI observes again. Refusals appear temporarily in the footer.

Known, named gap (#60/#61): the confirmation row's warnings line comes
from `PlanUnit::warnings` (dirty/unpushed/untracked/no-remote facts),
which predates the decision-evidence contract (#53) and is not yet
extended to render `PlanUnit::evidence`'s activity/consumer/recovery/
reclaimability facts inline. The CLI (`report --view external` text,
and the default/`--view external`/`--view agents` JSON) is the
currently-wired presentation surface for that contract -- see
`docs/usage.md`'s "Decision evidence" section. Revisit alongside the
`history_secs` simplification noted above the next time this file's
Actions/confirmation area is touched.

## Review

Review and deletion run on background workers. During an operation, replace the
confirmation row with a three-line progress area: processed/total group gauge,
success/refusal counts and current path, then phase-specific consequences.
Elapsed time advances even while one group is being checked. Never imply byte
reclamation progress. Unknown review totals show checked count rather than a
fabricated percentage. Esc/Ctrl-C/q request cancellation between groups, with a
visible Cancelling state; no second action starts while busy. Completed outcomes
are retained and refused/unattempted marks remain for explicit retry. Idle Ctrl-C
exits. Observation results are held while busy and pre-deletion results discarded
so a stale report cannot resurrect removed rows.

Keep the footer visible. Use overlays for help and the filter form, with inline action confirmation. Check empty results, narrow layouts, long paths, mixed filesystem/Docker selections, and missing history. The frame tests cover rendered text and layout; they do not establish readability on every font or color theme.
