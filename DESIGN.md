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

Collapsed build categories summarize nonempty, supported candidate groups with
count, allocated size, and oldest known modification age. Unknown age is `?`.
Candidate directory descendants are not counted again. Final outputs say
`Manual` with modification age, without suggesting a supported selective action.
Nested allocated sizes have a `*` suffix and a persistent accounting legend.
When space remains below the tree, preview the selected category's oldest
candidates with paths, allocated sizes, ages, and rebuilding effects. This preview
is read-only: expand the category to select exact groups.

Opening a project shows Cargo profiles and categories inside its build target.
Categories expand into exact groups in the same tree; category rows are navigation,
not selective cleanup units. The selected-row detail area shows recommendations
and rebuilding consequences. Compiler caches are a suggested starting point,
not a claim of obsolescence. No age-only or newest-hash-wins verdicts.

The change bar grows right for an increase and left for a decrease. Its length uses a logarithmic scale relative to visible changes. Changes below 1 MB use a small tick and dimmed text. The signed number supplies the actual value; the bar is not a linear scale of bytes.

Growth sorts descending by signed change. Other sorts cover size, name, ecosystem, and age. Tree traversal preserves hierarchy. Ecosystem glyphs follow project names; linked-worktree and build-output badges add context.

## Color

Growth is red, shrink is green, and secondary information is dim. The selected row uses a dark background (`Color::Indexed(236)`) and bold text. Marked rows use yellow and an `✗` prefix. Refusals use red text. Signed values and bar direction carry information independently of color.

The renderer uses terminal colors and an indexed selection color. Committed frames exercise 80×24 and 200×60 layouts; visual behavior on a particular terminal and palette still needs inspection.

## Navigation and filters

`→` opens or expands; `←` collapses or returns to projects. Enter opens a project or confirms an action. Esc cancels the active interaction or returns to projects. `/` opens the filter form; `:` edits the expression; `0` clears it. Parse errors retain the previous valid filter.

The initial filter is `growth > 100MB in 7d`. Saved filter and sort choices take precedence on later runs. Eight views are available through `v` and `1`–`8`. The [usage guide](docs/usage.md#terminal-controls) holds the full key table.

## Header, progress, and history

The header shows the root, observation status, available history, and totals as space permits. It drops trailing clauses on narrow terminals. A cached report can appear while an observation runs in the background; the first run needs an observation before it can display data.

Observation progress shows walked bytes and directories. Its percentage is an estimate against the previous walked total. A live FSEvents watch batches changes after 400 ms of quiet. The header can display a history sparkline; body rows use change bars.

## Actions

Space marks a row. Backspace opens the confirmation for the current row or marked set. Confirmation is a single inline row with selected paths, sizes, warnings, and destinations. Enter authorizes the action; Esc cancels it.

Project rows expand to actionable artifacts. If none exist, a direct project action may offer the checkout. Bulk marking with `A` skips that fallback. Worktree and source-directory selections carry their own warnings; the `ignored` and `untracked` summary buckets are not individual paths to delete.

Docker images and volumes must be named in the confirmation because their removal has no Trash recovery. Successful removals leave the displayed report, totals are adjusted, and the UI observes again. Refusals appear temporarily in the footer.

## Review

Keep the footer visible. Use overlays for help and the filter form, with inline action confirmation. Check empty results, narrow layouts, long paths, mixed filesystem/Docker selections, and missing history. The frame tests cover rendered text and layout; they do not establish readability on every font or color theme.
