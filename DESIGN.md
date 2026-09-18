# Design — slop-livin terminal UI

World: **diffstat ledger**. The screen traditions of the audience (git diff --stat, git log --graph, ncdu, k9s, htop) supply every element; nothing is invented for flavor. Mode: Operate.

## Grammar
- **Row = diffstat line.** `name  bytes  Δbytes  ▕bar▏  signals`. The bar is scaled to |growth| within the visible set, drawn with `+` for growth and `-` for shrink (`+++++++---`), never to size. Size is a number; growth is the picture.
- **Tree = graph rail.** Box-drawing rail (`├─ └─ │`) on the left, one indent level per depth: project → checkout/worktree → artifact → dir. Collapsed nodes show `▸`, expanded `▾`, count of hidden children in dim.
- **Filter line** at the top, editable with `/`: `growth > 100MB in 7d` (default on open). Grammar: `growth [><] <size> in <duration>`, plus `kind:<k>`, `project:<name>`, `merge-complete`, `idle > <dur>`. Parse errors show inline in red under the line; the previous filter stays applied.
- **Header** (one line): `root · observed 3m ago · since 7d · 63 projects · 41.6 GB attributed · 194 MB unowned · docker 15.8 GB unowned`.
- **Footer** (one line): key vocabulary, always visible: `↑↓ move  →/← expand  Enter open/confirm  ⌫ mark delete  / filter  v view  g growth-sort  s size-sort  ? help  q quit`.
- **Views** (`v` cycles, also `1–5`): projects · tree · kinds · docker · unowned. All render the same Report; nothing is computed only for the screen.

## Color (16-color safe; truecolor is a refinement, never a dependency)
- Default fg on terminal bg. Growth `+` green, shrink `-` red, unchanged dim. Selected row: reverse video. Marked-for-delete: yellow name + `✗` prefix — the only yellow on screen. Errors and refusals: red text, never a red background. No other color. Signals are dim; a fact that *blocks* deletion (occupied, dirty, unpushed) renders in default weight, not red — it is information, not alarm.

## Type & density
- Monospace is the medium, not a costume. Tabular alignment: numbers right-aligned in fixed columns, human units with one decimal (`1.2 GB`, `+175.5 MB`, `—` when no baseline). Names truncated with `…` in the middle, keeping the tail.
- Works at 80×24 (bar shrinks to 8 cells, signals drop to a single glyph column) and uses width up to 200 (bar grows, signals spell out).

## States
- Empty filter result: the filter line plus one line `no rows match — Backspace to widen, 0 to clear`; never a blank pane.
- Observing: header shows `observing… 38%` with the walk's directory count; the previous Report stays on screen (skeleton = last truth, never a spinner over nothing).
- Refusal at the sink: the row stays, the mark clears, the footer is replaced for 4 s by the cause and next step (`refused: activity changed since plan — press ⌫ again to re-plan`).
- Delete confirmation: inline, not modal — the marked rows collapse into a one-line summary above the footer: `delete 3 units · 1.9 GB → Trash · Enter confirm · Esc cancel`.
- Help (`?`): an overlay listing keys and the filter grammar; the only overlay in the product.

## Motion
- None decorative. The bar redraws on filter change; the marked-row collapse is instantaneous. Observation progress ticks in the header.

## Refuse
- Sunbursts, block-drawing pie charts, sparklines, colored backgrounds, emoji, ASCII logos, verdict words ("safe", "stale", "unused", "abandoned"), modals for anything but help.
