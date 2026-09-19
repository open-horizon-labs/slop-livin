//! Flattens a [`Report`] into displayable rows per view. Pure functions:
//! no I/O, no ratatui types, so this is unit-testable on its own.

use crate::filter::{self, Filter};
use crate::units::UnitId;
use std::collections::BTreeMap;
use swamp_core::report::{ArtifactKind, Report, UnownedReason};

/// Byte formatting is defined once, in core, so the TUI and the CLI can
/// never disagree about what "1.8GB" means (they did: one divided by 1024
/// under a decimal label while the other divided by 1000).
pub use swamp_core::render::human_bytes_pub as human_bytes;

pub use swamp_core::render::human_bytes_signed as human_signed_bytes;

/// Truncates `s` to `width` chars, keeping the tail: `foo…bar` rather
/// than `foo…`, per DESIGN.md ("truncated with `…` in the middle,
/// keeping the tail").
pub fn truncate_middle(s: &str, width: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= width || width < 4 {
        return s.to_string();
    }
    // Paths are recognised by their tail (`…/hiphi-repos/roon-knob`), so
    // keep almost all of the budget for it.
    let keep_tail = width - 1 - (width / 8).min(6);
    let keep_head = width - keep_tail - 1;
    let head: String = chars[..keep_head].iter().collect();
    let tail: String = chars[chars.len() - keep_tail..].iter().collect();
    format!("{head}…{tail}")
}

pub fn kind_label(kind: &ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::BuildOutput => "build",
        ArtifactKind::DependencyTree => "deps",
        ArtifactKind::Git => "git",
        ArtifactKind::Cache => "cache",
        ArtifactKind::Source => "source",
        ArtifactKind::Ignored => "ignored",
        ArtifactKind::Untracked => "untracked",
        ArtifactKind::DockerImage => "docker-image",
        ArtifactKind::DockerBuildCache => "docker-cache",
        ArtifactKind::DockerVolume => "docker-volume",
        ArtifactKind::Loose => "loose",
        ArtifactKind::Unknown => "unknown",
    }
}

/// `g` / `s`: sort the current view's rows by growth or by size. `None`
/// keeps report order (the tree view ignores sort -- its rows are a
/// hierarchy, not a flat ranked list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    #[default]
    None,
    Growth,
    Size,
    Name,
    /// Grouped by ecosystem (tags in table order), then by size.
    Type,
    /// Oldest first: time since the unit was last written.
    Age,
}

/// One renderable line: a diffstat row. `unit` is set when this row is a
/// single artifact that Backspace can act on (whether or not it is
/// currently markable — refusal is decided at mark time so the reason is
/// specific).
/// Facts a worktree row carries for the mark/remove decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeMark {
    pub path: std::path::PathBuf,
    pub linked: bool,
    /// The project's remote, when it has one: a whole checkout may only be
    /// archived if there is somewhere to restore it from.
    pub remote: Option<String>,
    pub dirty: Option<bool>,
    pub unpushed: Option<u32>,
    pub locked: Option<bool>,
    pub merge_complete: bool,
    pub pr: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub depth: usize,
    /// Box-drawing rail prefix (`├─ `, `└─ `, `│  `, ...) plus, for an
    /// expandable row, the collapse/expand glyph. Empty for a top-level
    /// (depth 0) row: those views have no tree to draw a rail for.
    pub rail: String,
    pub label: String,
    pub bytes: u64,
    pub growth: Option<i64>,
    /// Individual signal values (e.g. `"dirty"`, `"14m"`), rendered as a
    /// joined phrase at wide widths or a single glyph column at 80x24
    /// (DESIGN.md: "signals drop to a single glyph column").
    pub signals: Vec<String>,
    pub unit: Option<UnitId>,
    pub kind: Option<ArtifactKind>,
    /// Set on a worktree row: what Backspace needs to decide whether this
    /// worktree may be marked for removal, and the terms to record.
    pub worktree: Option<WorktreeMark>,
    /// git tracking status of this path, when known: tracked / ignored /
    /// untracked. Untracked bytes are in no version control and covered by
    /// no ignore rule — the fact that most changes what a human decides.
    pub track: Option<swamp_core::ignore::TrackState>,
    /// Byte history over the growth window, from the store (sparkline).
    pub series: Option<Vec<Option<u64>>>,
    /// Glyph badges drawn before the name: ecosystem glyphs, 🐳 when the
    /// project has Docker objects joined, 🔨 when it holds build output,
    /// ⎇ N for N linked worktrees. Empty for rows without facts to badge.
    pub badges: String,
    /// Ecosystem tags, for the type sort.
    pub ecosystems: Vec<String>,
    /// Newest mtime inside the unit (0 unknown), for the age sort/filter.
    pub mtime_max: u64,
    /// Present for a worktree row: how many artifact children are hidden
    /// because the row is collapsed.
    pub collapsed_children: Option<usize>,
    pub expandable: bool,
    /// Set on a projects-view row: the project's own name (not its
    /// display name), so marking can expand the row into that project's
    /// artifacts without parsing the rendered label back into an
    /// identity.
    pub project: Option<String>,
}

impl Row {
    fn leaf(depth: usize, label: String, bytes: u64, growth: Option<i64>) -> Self {
        Row {
            depth,
            rail: String::new(),
            label,
            bytes,
            growth,
            signals: Vec::new(),
            unit: None,
            kind: None,
            worktree: None,
            track: None,
            series: None,
            badges: String::new(),
            ecosystems: Vec::new(),
            mtime_max: 0,
            collapsed_children: None,
            expandable: false,
            project: None,
        }
    }
}

/// Display width of a string in terminal cells (emoji count as 2), the
/// only correct way to pad a column that holds glyph badges.
pub fn display_width(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}

/// Pads or truncates `s` to exactly `width` display cells.
pub fn pad_display(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w >= width {
        let mut out = String::new();
        let mut used = 0;
        for ch in s.chars() {
            let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + cw > width {
                break;
            }
            out.push(ch);
            used += cw;
        }
        out.push_str(&" ".repeat(width - used));
        return out;
    }
    format!("{s}{}", " ".repeat(width - w))
}

/// The badge string for a set of ecosystem tags plus project facts.
pub fn badges(
    ecosystems: &[String],
    docker: bool,
    builds: bool,
    linked_worktrees: usize,
) -> String {
    let mut b: String = ecosystems
        .iter()
        .take(3)
        .map(|t| swamp_core::ecosystem::glyph_for(t))
        .collect();
    if docker && !ecosystems.iter().any(|t| t == "docker") {
        b.push('🐳');
    }
    if builds {
        b.push('🔨');
    }
    if linked_worktrees > 0 {
        if !b.is_empty() {
            b.push(' ');
        }
        // Give the branching glyph breathing room: terminal fonts can
        // overhang its cell and crowd the first digit, especially in bold.
        b.push_str(&format!("⎇ {linked_worktrees}"));
    }
    b
}

fn passes_filter(growth: Option<i64>, filter: &Filter) -> bool {
    filter::growth_passes(filter, growth)
}

/// Bytes below which a change is noise on a disk of this size: drawn as
/// a tick and never a bar, and the number beside it is dimmed.
pub const NOISE_FLOOR: i64 = 1_000_000;

/// One row's change as a diverging bar around a fixed centre axis:
/// shrink extends left, growth extends right, so the direction is the
/// geometry and the colour only reinforces it. Length is logarithmic
/// over `max_abs`, because a linear scale across four orders of
/// magnitude renders everything below the largest row as the same
/// one-cell sliver — 107 MB and 3 MB looked identical while 13.6 GB
/// filled the column.
///
/// Returns `(left, axis, right)`: the caller styles the two sides
/// separately. Each side is `half` cells wide; `axis` is one cell.
pub fn diverging_bar(growth: Option<i64>, max_abs: i64, half: usize) -> (String, char, String) {
    // Eighth-blocks for the right side, which fills away from the axis,
    // and one half-block for the left, which fills toward it. Only
    // U+2580..U+259F here: the U+1FB8x "eighth block" range that would
    // mirror the steps exactly is Unicode 13 and renders as tofu in many
    // terminals, which is worse than a coarser left edge.
    const STEPS: [char; 8] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
    const PARTIAL_L: char = '▐';
    let blank = " ".repeat(half);
    let g = growth.unwrap_or(0);
    if half == 0 {
        return (String::new(), '│', String::new());
    }
    if g == 0 || max_abs <= 0 {
        return (blank.clone(), '│', blank);
    }
    let magnitude = g.unsigned_abs() as f64;
    let grew = g > 0;

    // Below the floor: one tick hugging the axis, never scaled. A change
    // nobody would act on must not look like one that someone would.
    if magnitude < NOISE_FLOOR as f64 {
        return if grew {
            (blank, '│', STEPS[0].to_string() + &" ".repeat(half - 1))
        } else {
            (" ".repeat(half - 1) + &PARTIAL_L.to_string(), '│', blank)
        };
    }

    // log1p over the range, so the floor is a visible nub rather than
    // nothing and the largest change is exactly full.
    let span =
        ((max_abs.max(NOISE_FLOOR) as f64).ln() - (NOISE_FLOOR as f64).ln()).max(f64::EPSILON);
    let frac = ((magnitude.ln() - (NOISE_FLOOR as f64).ln()) / span).clamp(0.0, 1.0);
    let eighths = ((frac * half as f64) * 8.0).round().max(1.0) as usize;
    let full = (eighths / 8).min(half);
    let rem = if full == half { 0 } else { eighths % 8 };

    if grew {
        let mut bar = "█".repeat(full);
        if rem > 0 {
            bar.push(STEPS[rem - 1]);
        }
        let pad = half.saturating_sub(bar.chars().count());
        (blank, '│', bar + &" ".repeat(pad))
    } else {
        // Anchored at the axis: the partial cell is the outer one, inked
        // on its inner edge so the bar stays continuous.
        let partial = if rem > 0 { 1 } else { 0 };
        let pad = half.saturating_sub(full + partial);
        let mut bar = " ".repeat(pad);
        if partial == 1 {
            bar.push(PARTIAL_L);
        }
        bar.push_str(&"█".repeat(full));
        (bar, '│', blank)
    }
}

/// Whether this change is below the noise floor, so the row's number can
/// be dimmed with it.
pub fn is_noise(growth: Option<i64>) -> bool {
    growth.is_some_and(|g| g != 0 && g.unsigned_abs() < NOISE_FLOOR as u64)
}

/// Per-bucket change of a byte series: `series[i] - series[i-1]`, `None`
/// where either side was unobserved. This is what a row's sparkline
/// draws: *when* bytes moved and how much, not how big the tree is (a
/// 16 GB tree drawn as size is a solid brick that says nothing).
pub fn deltas(series: &[Option<u64>]) -> Vec<Option<i64>> {
    series
        .windows(2)
        .map(|w| match (w[0], w[1]) {
            (Some(a), Some(b)) => Some(b as i64 - a as i64),
            _ => None,
        })
        .collect()
}

/// Downsample deltas to `width` points. Each point is the bin's change
/// of largest magnitude, sign kept, so a spike survives; a bin with no
/// observation stays `None`.
pub fn spark_deltas(series: &[Option<u64>], width: usize) -> Vec<Option<i64>> {
    let d = deltas(series);
    if d.is_empty() || width == 0 {
        return Vec::new();
    }
    if d.len() <= width {
        return d;
    }
    (0..width)
        .map(|i| {
            let lo = i * d.len() / width;
            let hi = ((i + 1) * d.len() / width).max(lo + 1);
            d[lo..hi.min(d.len())]
                .iter()
                .filter_map(|v| *v)
                .max_by_key(|v| v.unsigned_abs())
        })
        .collect()
}

/// A series is flat when nothing moved between any two observations:
/// nothing worth a glyph.
pub fn is_flat(series: &[Option<u64>]) -> bool {
    deltas(series).iter().all(|d| d.is_none_or(|d| d == 0))
}

/// Net change over the observed part of a series: last minus first
/// observed value. `None` with fewer than two observations.
pub fn net_change(series: &[Option<u64>]) -> Option<i64> {
    let mut it = series.iter().filter_map(|v| *v);
    let first = it.next()? as i64;
    let last = it.next_back()? as i64;
    Some(last - first)
}

/// Trend of a series: +1 rising, -1 falling, 0 flat (first vs last observed).
pub fn trend(series: &[Option<u64>]) -> i8 {
    match net_change(series) {
        Some(d) if d > 0 => 1,
        Some(d) if d < 0 => -1,
        _ => 0,
    }
}

pub fn max_abs_growth<'a>(rows: impl Iterator<Item = &'a Row>) -> i64 {
    rows.filter_map(|r| r.growth)
        .map(|g| g.abs())
        .max()
        .unwrap_or(0)
}

/// Applies `sort` to a flat (non-hierarchical) row list. Growth and size
/// sort largest first, name alphabetical, type grouped by first ecosystem
/// tag in table order then size, age oldest first (unknown age last). A
/// stable sort keeps report order as the tiebreak; `reverse` flips the
/// whole order.
pub fn apply_sort(rows: &mut [Row], sort: Sort, reverse: bool) {
    fn type_rank(r: &Row) -> usize {
        r.ecosystems
            .first()
            .and_then(|t| {
                swamp_core::ecosystem::ECOSYSTEMS
                    .iter()
                    .position(|e| e.tag == t)
            })
            .unwrap_or(usize::MAX)
    }
    match sort {
        Sort::None => {}
        // Signed, not by magnitude: the question "what grew" is answered
        // by what arrived, and bytes that left are the opposite of the
        // answer. Sorting by magnitude put a project that shrank by 3GB
        // above one that grew by 1GB, at the top of a screen the human
        // is reading for things to delete. Shrinkage sorts last.
        Sort::Growth => {
            rows.sort_by(|a, b| b.growth.unwrap_or(0).cmp(&a.growth.unwrap_or(0)));
        }
        Sort::Size => rows.sort_by(|a, b| b.bytes.cmp(&a.bytes)),
        Sort::Name => rows.sort_by(|a, b| {
            // Labels may carry ecosystem tags ("[rs][js] owner/repo");
            // sort on the name after the last tag so tags don't cluster rows.
            let key = |l: &str| l.rsplit("] ").next().unwrap_or(l).to_lowercase();
            key(&a.label).cmp(&key(&b.label))
        }),
        Sort::Type => rows.sort_by(|a, b| {
            type_rank(a)
                .cmp(&type_rank(b))
                .then_with(|| b.bytes.cmp(&a.bytes))
        }),
        Sort::Age => rows.sort_by(|a, b| {
            let key = |r: &Row| {
                if r.mtime_max == 0 {
                    u64::MAX
                } else {
                    r.mtime_max
                }
            };
            key(a).cmp(&key(b))
        }),
    }
    if reverse && sort != Sort::None {
        rows.reverse();
    }
}

/// Projects view: one row per project, aggregated bytes/growth.
pub fn projects_rows(report: &Report, filter: &Filter) -> Vec<Row> {
    let mut out = Vec::new();
    for p in &report.projects {
        if let Some(name) = filter::project_name(filter)
            && !p
                .name
                .to_ascii_lowercase()
                .contains(&name.to_ascii_lowercase())
        {
            continue;
        }
        // `type:` predicates are facts about the project.
        if !filter::type_passes(filter, p) {
            continue;
        }
        // Worktree-level predicates (idle, merge-complete, pr) hold for a
        // project when at least one of its worktrees satisfies them.
        if filter::has_worktree_predicates(filter)
            && !p.worktrees.iter().any(|wt| worktree_passes(filter, wt))
        {
            continue;
        }
        let mut bytes = 0u64;
        let mut growth = 0i64;
        let mut have_growth = false;
        for wt in &p.worktrees {
            for a in &wt.artifacts {
                bytes += a.bytes;
                if let Some(g) = a.growth_bytes {
                    growth += g;
                    have_growth = true;
                }
            }
        }
        let growth = have_growth.then_some(growth);
        if !passes_filter(growth, filter) || !filter::size_passes(filter, bytes) {
            continue;
        }
        let mtime_max = p
            .worktrees
            .iter()
            .flat_map(|wt| wt.artifacts.iter())
            .map(|a| a.mtime_max)
            .max()
            .unwrap_or(0);
        if filter::has_age_predicate(filter)
            && !p
                .worktrees
                .iter()
                .flat_map(|wt| wt.artifacts.iter())
                .any(|a| filter::age_passes(filter, a.mtime_max))
        {
            continue;
        }
        let docker = p
            .worktrees
            .iter()
            .flat_map(|wt| wt.artifacts.iter())
            .any(|a| {
                matches!(
                    a.kind,
                    ArtifactKind::DockerImage
                        | ArtifactKind::DockerBuildCache
                        | ArtifactKind::DockerVolume
                )
            });
        let builds = p
            .worktrees
            .iter()
            .flat_map(|wt| wt.artifacts.iter())
            .any(|a| a.kind == ArtifactKind::BuildOutput);
        let linked = p
            .worktrees
            .iter()
            .filter(|w| w.kind == swamp_core::report::WorktreeKind::Linked)
            .count();
        let series = sum_series(p.worktrees.iter().flat_map(|wt| {
            wt.artifacts.iter().filter_map(move |a| {
                let rel = a
                    .path
                    .strip_prefix(&wt.path)
                    .map(|r| r.display().to_string())
                    .unwrap_or_default();
                report.series_by_key.get(&swamp_core::growth::series_key(
                    &p.project_id,
                    &wt.worktree_id,
                    &format!("{:?}", a.kind),
                    &rel,
                ))
            })
        }));
        out.push(Row {
            depth: 0,
            rail: String::new(),
            label: project_display_name(p),
            bytes,
            growth,
            signals: Vec::new(),
            unit: None,
            kind: None,
            worktree: None,
            track: None,
            series,
            badges: badges(&p.ecosystems, docker, builds, linked),
            ecosystems: p.ecosystems.clone(),
            mtime_max,
            collapsed_children: None,
            expandable: true,
            project: Some(p.name.clone()),
        });
    }
    out
}

/// Tree view for one project: checkout/worktree -> (folded) artifact
/// rows, built from the same [`swamp_core::tree::build_project_tree`]
/// the CLI's `--project` drill uses, so the two never drift apart (#33).
/// This function only adds the TUI-specific rail glyphs
/// (`├─`/`└─`/`│`, DESIGN.md's graph-rail grammar), the collapse/expand
/// state, and unit ids for marking. `collapsed` names worktree paths (as
/// strings) currently collapsed.
pub fn tree_rows(
    report: &Report,
    project_name: &str,
    filter: &Filter,
    collapsed: &std::collections::HashSet<String>,
    track: &std::collections::HashMap<std::path::PathBuf, swamp_core::ignore::TrackState>,
) -> Vec<Row> {
    let mut out = Vec::new();
    let Some(p) = report.projects.iter().find(|p| p.name == project_name) else {
        return out;
    };
    let tree = swamp_core::tree::build_project_tree(p, &report.root);
    let wt_count = tree.worktrees.len();
    for (wi, wt) in tree.worktrees.iter().enumerate() {
        // Look up the underlying `WorktreeRow` for its absolute path (the
        // tree model's `rel_path` is relative, but marking/collapse keys
        // and folded-row unit ids need a real path on disk).
        let Some(source_wt) = p.worktrees.iter().find(|w| w.worktree_id == wt.worktree_id) else {
            continue;
        };
        if !worktree_passes(filter, source_wt) {
            continue;
        }
        let wt_last = wi + 1 == wt_count;
        let wt_key = source_wt.path.display().to_string();
        let is_collapsed = collapsed.contains(&wt_key);
        let signals: Vec<String> = wt.signals.iter().map(|s| s.value.clone()).collect();
        let wt_connector = if wt_last { "└─ " } else { "├─ " };
        let expand_glyph = if is_collapsed { "▸" } else { "▾" };
        let raw = source_wt.raw_signals();
        let mark = WorktreeMark {
            path: source_wt.path.clone(),
            linked: matches!(wt.kind, swamp_core::report::WorktreeKind::Linked),
            remote: p.remote.clone(),
            dirty: raw.0,
            unpushed: raw.1,
            locked: raw.2,
            merge_complete: source_wt
                .merge_complete
                .as_ref()
                .is_some_and(|m| m.verdict == swamp_core::github::TriState::Yes),
            pr: source_wt
                .github
                .as_ref()
                .and_then(|g| match &g.pull_request {
                    swamp_core::github::PrStatus::Some(pr) => {
                        Some(format!("PR #{} {:?}", pr.number, pr.state).to_lowercase())
                    }
                    _ => None,
                }),
        };
        let wt_series = sum_series(source_wt.artifacts.iter().filter_map(|a| {
            let rel = a
                .path
                .strip_prefix(&source_wt.path)
                .map(|r| r.display().to_string())
                .unwrap_or_default();
            report.series_by_key.get(&swamp_core::growth::series_key(
                &p.project_id,
                &source_wt.worktree_id,
                &format!("{:?}", a.kind),
                &rel,
            ))
        }));
        out.push(Row {
            depth: 1,
            rail: format!("{wt_connector}{expand_glyph} "),
            label: format!(
                "{} {}",
                format!("{:?}", wt.kind).to_lowercase(),
                source_wt.path.display()
            ),
            bytes: wt.bytes,
            growth: wt.growth_bytes,
            signals,
            unit: Some(UnitId::for_artifact(&source_wt.path)),
            kind: None,
            worktree: Some(mark),
            track: None,
            series: wt_series,
            badges: String::new(),
            ecosystems: Vec::new(),
            mtime_max: 0,
            collapsed_children: is_collapsed.then_some(wt.rows.len()),
            expandable: !wt.rows.is_empty(),
            project: None,
        });
        if is_collapsed {
            continue;
        }
        let child_prefix = if wt_last { "   " } else { "│  " };
        let visible: Vec<&swamp_core::tree::TreeRow> = wt
            .rows
            .iter()
            .filter(|row| {
                filter::kind_passes(filter, row.kind_label, row.kind.as_ref())
                    && passes_filter(row.growth_bytes, filter)
                    && filter::size_passes(filter, row.bytes)
                    && (row.kind_label == "source" || filter::age_passes(filter, row.mtime_max))
            })
            .collect();
        let n = visible.len();
        for (ri, row) in visible.into_iter().enumerate() {
            let r_last = ri + 1 == n;
            let connector = if r_last { "└─ " } else { "├─ " };
            let label = if row.folded_count > 1 {
                format!(
                    "{} {} (x{})",
                    row.kind_label, row.rel_path, row.folded_count
                )
            } else {
                format!("{} {}", row.kind_label, row.rel_path)
            };
            let abs = source_wt.path.join(&row.rel_path);
            let series_key = swamp_core::growth::series_key(
                &p.project_id,
                &source_wt.worktree_id,
                &row.kind
                    .as_ref()
                    .map(|k| format!("{k:?}"))
                    .unwrap_or_default(),
                &row.rel_path,
            );
            let series = report.series_by_key.get(&series_key).cloned();
            let is_source = row.kind_label == "source";
            let source_key = format!("source:{}", source_wt.path.display());
            let source_collapsed = collapsed.contains(&source_key);
            let children: Vec<&swamp_core::report::DirRollup> = if is_source {
                source_children(report, &source_wt.worktree_id)
            } else {
                Vec::new()
            };
            let mut out_row = Row::leaf(2, label, row.bytes, row.growth_bytes);
            out_row.rail = format!(
                "{child_prefix}{connector}{}",
                if is_source && !children.is_empty() {
                    if source_collapsed { "▸ " } else { "▾ " }
                } else {
                    ""
                }
            );
            out_row.kind = row.kind.clone();
            out_row.series = series;
            out_row.mtime_max = row.mtime_max;
            if let Some(t) = &row.ecosystem {
                out_row.badges = swamp_core::ecosystem::glyph_for(t).to_string();
                out_row.ecosystems = vec![t.clone()];
            }
            // `.git` is git's own store, not content it tracks: annotating
            // it "untracked" is noise, so it carries no status.
            out_row.track = (row.kind_label != "git")
                .then(|| track.get(&abs).copied())
                .flatten();
            out_row.expandable = is_source && !children.is_empty();
            out_row.collapsed_children = (is_source && source_collapsed).then_some(children.len());
            // A folded group of several artifacts has no single owning
            // path to mark; only an unfolded row is markable.
            if row.folded_count == 1 {
                out_row.unit = Some(UnitId::for_artifact(&abs));
            }
            out.push(out_row);
            // A Source tree is one row only because nothing inside it is a
            // classified artifact -- which is exactly when its contents are
            // worth seeing. Expanded, it lists its own top-level
            // directories with their git tracking status.
            if is_source && !source_collapsed {
                let shown = children.len().min(8);
                for (ci, d) in children.iter().take(shown).enumerate() {
                    let c_last = ci + 1 == shown && children.len() <= shown;
                    let c_connector = if c_last { "└─ " } else { "├─ " };
                    let dir_abs = source_wt.path.join(&d.rel_path);
                    let mut child =
                        Row::leaf(3, format!("dir {}", d.rel_path), d.allocated_total, None);
                    child.rail = format!("{child_prefix}   {c_connector}");
                    child.track = track.get(&dir_abs).copied();
                    child.unit = Some(UnitId::for_artifact(&dir_abs));
                    child.kind = Some(swamp_core::report::ArtifactKind::Unknown);
                    out.push(child);
                }
                if children.len() > shown {
                    let rest: u64 = children.iter().skip(shown).map(|d| d.allocated_total).sum();
                    let mut more = Row::leaf(
                        3,
                        format!("… and {} more directories", children.len() - shown),
                        rest,
                        None,
                    );
                    more.rail = format!("{child_prefix}   └─ ");
                    out.push(more);
                }
            }
        }
    }
    out
}

pub use swamp_core::render::project_display_name;

/// Top-level directories of a worktree's Source tree, biggest first.
/// Empty when the report was built without directory rollups.
fn source_children<'a>(
    report: &'a Report,
    worktree_id: &str,
) -> Vec<&'a swamp_core::report::DirRollup> {
    let Some(dirs) = report
        .dirs_by_worktree
        .as_ref()
        .and_then(|m| m.get(worktree_id))
    else {
        return Vec::new();
    };
    let mut top: Vec<&swamp_core::report::DirRollup> = dirs
        .iter()
        .filter(|d| !d.rel_path.is_empty() && d.rel_path != "." && !d.rel_path.contains('/'))
        .collect();
    top.sort_by(|a, b| b.allocated_total.cmp(&a.allocated_total));
    top
}

/// Kinds view: bytes/count per artifact kind across the whole root.
pub fn kinds_rows(report: &Report, filter: &Filter) -> Vec<Row> {
    let mut totals: BTreeMap<String, (u64, i64, u32)> = BTreeMap::new();
    for p in &report.projects {
        for wt in &p.worktrees {
            for a in &wt.artifacts {
                let e = totals.entry(kind_label(&a.kind).to_string()).or_default();
                e.0 += a.bytes;
                e.1 += a.growth_bytes.unwrap_or(0);
                e.2 += 1;
            }
        }
    }
    totals
        .into_iter()
        .filter(|(k, _)| filter::kind_passes(filter, k, None))
        .map(|(k, (bytes, growth, count))| Row {
            depth: 0,
            rail: String::new(),
            label: format!("{k} ({count})"),
            bytes,
            growth: Some(growth),
            signals: Vec::new(),
            unit: None,
            kind: None,
            worktree: None,
            track: None,
            series: None,
            badges: String::new(),
            ecosystems: Vec::new(),
            mtime_max: 0,
            collapsed_children: None,
            expandable: false,
            project: None,
        })
        .collect()
}

/// Bytes belonging to Docker objects with no join evidence
/// (`UnownedReason::DockerNoJoin`), reported separately from the rest of
/// unowned per PRODUCT.md's honest-coverage principle.
pub fn docker_unowned_bytes(report: &Report) -> u64 {
    report
        .unowned
        .iter()
        .filter(|u| u.reason == UnownedReason::DockerNoJoin)
        .map(|u| u.bytes)
        .sum()
}

/// Docker view: unowned docker rows plus a per-project docker rollup.
pub fn docker_rows(report: &Report) -> Vec<Row> {
    let mut out = Vec::new();
    for p in &report.projects {
        for wt in &p.worktrees {
            for a in &wt.artifacts {
                if !matches!(
                    a.kind,
                    ArtifactKind::DockerImage
                        | ArtifactKind::DockerBuildCache
                        | ArtifactKind::DockerVolume
                ) {
                    continue;
                }
                let mut row = Row::leaf(
                    0,
                    format!("{} · {} {}", p.name, kind_label(&a.kind), a.path.display()),
                    a.bytes,
                    a.growth_bytes,
                );
                row.kind = Some(a.kind.clone());
                row.unit = Some(UnitId::for_artifact(&a.path));
                out.push(row);
            }
        }
    }
    for u in &report.unowned {
        if u.reason != UnownedReason::DockerNoJoin {
            continue;
        }
        let mut row = Row::leaf(0, format!("unowned · {}", u.path_or_object), u.bytes, None);
        // An unjoined object is still a real object: it can be acted on,
        // it just belongs to no project. The kind decides what happens.
        row.kind = Some(match u.docker_kind.as_deref() {
            Some("volume") => ArtifactKind::DockerVolume,
            Some("build-cache") => ArtifactKind::DockerBuildCache,
            _ => ArtifactKind::DockerImage,
        });
        row.unit = Some(UnitId::for_artifact(std::path::Path::new(
            &u.path_or_object,
        )));
        out.push(row);
    }
    out
}

/// Builds view: every `BuildOutput`/`Cache` row across the whole root,
/// same kind set as the CLI's `--view builds` (#33).
pub fn builds_rows(report: &Report, filter: &Filter) -> Vec<Row> {
    let mut rows = kind_filtered_rows(
        report,
        &[ArtifactKind::BuildOutput, ArtifactKind::Cache],
        filter,
    );
    append_cargo_breakdowns(report, filter, &mut rows);
    rows
}

/// Deps view: every `DependencyTree` row across the whole root, same as
/// the CLI's `--view deps`.
pub fn deps_rows(report: &Report, filter: &Filter) -> Vec<Row> {
    kind_filtered_rows(report, &[ArtifactKind::DependencyTree], filter)
}

fn kind_filtered_rows(report: &Report, kinds: &[ArtifactKind], filter: &Filter) -> Vec<Row> {
    let mut out = Vec::new();
    for p in &report.projects {
        if !filter::type_passes(filter, p) {
            continue;
        }
        if let Some(name) = filter::project_name(filter)
            && !swamp_core::filter::name_matches(name, &p.name)
        {
            continue;
        }
        for wt in &p.worktrees {
            for a in &wt.artifacts {
                if !kinds.contains(&a.kind)
                    || !filter::size_passes(filter, a.bytes)
                    || !filter::age_passes(filter, a.mtime_max)
                    || !passes_filter(a.growth_bytes, filter)
                {
                    continue;
                }
                let mut row = Row::leaf(
                    0,
                    format!(
                        "{} · {} {}",
                        project_display_name(p),
                        kind_label(&a.kind),
                        a.path.display()
                    ),
                    a.bytes,
                    a.growth_bytes,
                );
                row.kind = Some(a.kind.clone());
                row.unit = Some(UnitId::for_artifact(&a.path));
                row.mtime_max = a.mtime_max;
                if let Some(t) = &a.ecosystem {
                    row.badges = swamp_core::ecosystem::glyph_for(t).to_string();
                    row.ecosystems = vec![t.clone()];
                }
                out.push(row);
            }
        }
    }
    out
}

/// Adds a compact, non-actionable Cargo breakdown below build rows.  The
/// common build row remains the accounting/authorization boundary; these
/// aggregate children explain the logical storage without turning the TUI
/// into a second cleanup authority or rendering every hashed leaf.
fn append_cargo_breakdowns(report: &Report, filter: &Filter, rows: &mut Vec<Row>) {
    let mut additions = Vec::new();
    for p in &report.projects {
        if !filter::type_passes(filter, p) {
            continue;
        }
        if let Some(name) = filter::project_name(filter)
            && !swamp_core::filter::name_matches(name, &p.name)
        {
            continue;
        }
        for wt in &p.worktrees {
            for a in &wt.artifacts {
                if a.kind != ArtifactKind::BuildOutput {
                    continue;
                }
                let Some(parent_index) = rows.iter().position(|r| {
                    r.kind.as_ref() == Some(&ArtifactKind::BuildOutput)
                        && r.unit == Some(UnitId::for_artifact(&a.path))
                }) else {
                    continue;
                };
                let mut children: Vec<_> = report
                    .nested_artifacts
                    .iter()
                    .filter(|u| u.path != a.path && u.path.starts_with(&a.path))
                    .filter(|u| {
                        matches!(
                            u.role,
                            swamp_core::artifact::ArtifactRole::Profile
                                | swamp_core::artifact::ArtifactRole::Dependency
                                | swamp_core::artifact::ArtifactRole::Example
                                | swamp_core::artifact::ArtifactRole::BuildScriptOutput
                                | swamp_core::artifact::ArtifactRole::Incremental
                                | swamp_core::artifact::ArtifactRole::Residual
                                | swamp_core::artifact::ArtifactRole::TestExecutable
                        )
                    })
                    .filter(|u| {
                        u.path
                            .strip_prefix(&a.path)
                            .map(|p| {
                                p.components().count() <= 2
                                    || swamp_core::cargo_cleanup::candidate(u)
                                    || u.role == swamp_core::artifact::ArtifactRole::TestExecutable
                                    || (!u.is_dir
                                        && u.role == swamp_core::artifact::ArtifactRole::Example)
                            })
                            .unwrap_or(false)
                    })
                    .collect();
                children.sort_by_key(|u| (u.path.components().count(), u.path.clone()));
                let additions_for_row: Vec<_> = children
                    .into_iter()
                    .map(|u| {
                        let mut row = Row::leaf(
                            1,
                            format!(
                                "  cargo · {} · {} (physical {}){}",
                                u.role.label(),
                                u.path.display(),
                                human_bytes(u.physical_total),
                                if u.variant.unknowns.is_empty() {
                                    String::new()
                                } else {
                                    " · unknowns".to_string()
                                }
                            ),
                            u.bytes,
                            u.growth_bytes,
                        );
                        row.mtime_max = u.mtime_max;
                        row.series = report
                            .series_by_key
                            .get(&format!("Nested:{}", u.id))
                            .cloned();
                        if swamp_core::cargo_cleanup::candidate(u) {
                            row.unit = Some(UnitId::for_artifact(&u.path));
                            row.kind = Some(ArtifactKind::BuildOutput);
                            row.signals = vec!["review exact group".into()];
                        } else {
                            row.signals = if u.coverage.supported {
                                vec!["inspection-only".into()]
                            } else {
                                vec!["coverage-limited".into()]
                            };
                        }
                        row
                    })
                    .collect();
                additions.push((parent_index + 1, additions_for_row));
            }
        }
    }
    for (index, mut children) in additions.into_iter().rev() {
        rows.splice(index..index, children.drain(..));
    }
}

/// Types view: one row per ecosystem, with the projects wearing the tag
/// and the bytes/growth of the artifacts it generates (`Report.summary`).
pub fn types_rows(report: &Report, filter: &Filter) -> Vec<Row> {
    let mut rows: Vec<Row> = report
        .summary
        .by_type
        .iter()
        .filter(|(tag, _)| {
            filter::type_passes(
                filter,
                &swamp_core::report::ProjectRow {
                    project_id: String::new(),
                    name: String::new(),
                    worktrees: Vec::new(),
                    ecosystems: vec![(*tag).clone()],
                    remote: None,
                },
            )
        })
        .map(|(tag, t)| {
            let mut row = Row::leaf(
                0,
                format!(
                    "{} · {} project{} · {} artifact{}",
                    t.name,
                    t.projects,
                    if t.projects == 1 { "" } else { "s" },
                    t.artifacts,
                    if t.artifacts == 1 { "" } else { "s" }
                ),
                t.bytes,
                t.growth_bytes,
            );
            row.badges = if tag == "other" {
                String::new()
            } else {
                swamp_core::ecosystem::glyph_for(tag).to_string()
            };
            row.ecosystems = vec![tag.clone()];
            row
        })
        .collect();
    rows.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    rows
}

/// Unowned view.
pub fn unowned_rows(report: &Report) -> Vec<Row> {
    report
        .unowned
        .iter()
        .filter(|u| u.reason != UnownedReason::DockerNoJoin)
        .map(|u| {
            let mut row = Row::leaf(
                0,
                format!("{:?} · {}", u.reason, u.path_or_object),
                u.bytes,
                None,
            );
            // Bytes nothing claims are still bytes, and the path is real:
            // it can be marked like any other unit and goes to Trash.
            // Except one the walk could not even read — there is nothing
            // to stand behind.
            if u.reason != UnownedReason::PermissionDenied {
                row.kind = Some(ArtifactKind::Loose);
                row.unit = Some(UnitId::for_artifact(std::path::Path::new(
                    &u.path_or_object,
                )));
            }
            row
        })
        .collect()
}

/// Element-wise sum of several equal-length series; `None` if none.
/// Elementwise sum of child series. A bucket is `None` only when no child
/// had been observed yet at that time.
fn sum_series<'a>(it: impl Iterator<Item = &'a Vec<Option<u64>>>) -> Option<Vec<Option<u64>>> {
    let mut acc: Option<Vec<Option<u64>>> = None;
    for s in it {
        match acc.as_mut() {
            None => acc = Some(s.clone()),
            Some(a) => {
                for (x, y) in a.iter_mut().zip(s.iter()) {
                    if let Some(y) = y {
                        *x = Some(x.unwrap_or(0) + y);
                    }
                }
            }
        }
    }
    acc
}

/// Worktree-level predicates against a report row's facts.
fn worktree_passes(filter: &Filter, wt: &swamp_core::report::WorktreeRow) -> bool {
    let merge_complete = wt
        .merge_complete
        .as_ref()
        .is_some_and(|m| m.verdict == swamp_core::github::TriState::Yes);
    let pr_state = wt.github.as_ref().and_then(|g| match &g.pull_request {
        swamp_core::github::PrStatus::Some(pr) => Some(&pr.state),
        _ => None,
    });
    filter::worktree_passes(filter, wt.idle_secs, merge_complete, pr_state)
}

/// Parses the rendered worktree signals back into facts for the mark
/// decision: (dirty, unpushed, locked). `None` = unknown.
trait RawWorktreeSignals {
    fn raw_signals(&self) -> (Option<bool>, Option<u32>, Option<bool>);
}
impl RawWorktreeSignals for swamp_core::report::WorktreeRow {
    fn raw_signals(&self) -> (Option<bool>, Option<u32>, Option<bool>) {
        let mut dirty = None;
        let mut unpushed = None;
        let mut locked = None;
        for s in &self.signals {
            match s.name.as_str() {
                "dirty" => {
                    dirty = match s.value.as_str() {
                        "dirty" => Some(true),
                        "clean" => Some(false),
                        _ => None,
                    }
                }
                "unpushed" => {
                    unpushed = s
                        .value
                        .split_whitespace()
                        .next()
                        .and_then(|n| n.parse::<u32>().ok());
                }
                "locked" => {
                    locked = match s.value.as_str() {
                        "locked" => Some(true),
                        "unlocked" => Some(false),
                        _ => None,
                    }
                }
                _ => {}
            }
        }
        (dirty, unpushed, locked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_badges_separate_glyphs_from_multi_digit_counts() {
        assert_eq!(badges(&[], false, false, 0), "");
        assert_eq!(badges(&[], false, true, 0), "🔨");
        for count in [1, 9, 10, 12, 99, 100] {
            assert_eq!(badges(&[], false, false, count), format!("⎇ {count}"));
            let badge = badges(&[], false, true, count);
            assert_eq!(badge, format!("🔨 ⎇ {count}"));
            assert_eq!(display_width(&badge), 5 + count.to_string().len());
        }
    }

    #[test]
    fn human_bytes_formats_units() {
        // Decimal, matching the SI labels the product prints (a GB is
        // 1_000_000_000 bytes, not a GiB under a GB label).
        assert_eq!(human_bytes(500), "500B");
        assert_eq!(human_bytes(1536), "1.5KB");
        assert_eq!(human_bytes(1_288_490_188), "1.3GB");
        assert_eq!(human_bytes(1_000_000_000), "1.0GB");
        // The exact figure behind the reported arithmetic bug.
        assert_eq!(human_bytes(1_951_580_160), "2.0GB");
    }

    #[test]
    fn signed_bytes_show_sign_and_dash() {
        assert_eq!(human_signed_bytes(0), "0B");
        assert_eq!(human_signed_bytes(184_320_000), "+184.3MB");
        assert_eq!(human_signed_bytes(-1024), "-1.0KB");
    }

    #[test]
    fn diverging_bar_puts_direction_in_the_geometry() {
        let max = 10_000_000_000;
        let (l, axis, r) = diverging_bar(Some(max), max, 8);
        assert_eq!(axis, '│');
        assert_eq!(r, "████████", "the largest growth fills the right side");
        assert_eq!(l, "        ", "and leaves the left side empty");
        let (l, _, r) = diverging_bar(Some(-max), max, 8);
        assert_eq!(l, "████████", "shrink of the same size fills the left");
        // The left bar hugs the axis: its padding is on the outside.
        let (l, _, _) = diverging_bar(Some(-107_000_000), max, 8);
        assert!(l.starts_with(' ') && l.ends_with('█'), "{l:?}");
        assert_eq!(r, "        ");
        let (l, _, r) = diverging_bar(None, max, 8);
        assert_eq!((l.trim(), r.trim()), ("", ""), "no measurement, no bar");
        assert_eq!(diverging_bar(Some(0), max, 8).2.trim(), "");
    }

    #[test]
    fn a_log_scale_separates_the_sizes_a_linear_one_flattened() {
        // The frame that prompted this: 13.6GB, 107MB and 3MB shared a
        // column, and the last two were the same single sliver.
        let max = 13_600_000_000;
        let big = diverging_bar(Some(13_600_000_000), max, 20)
            .2
            .trim_end()
            .chars()
            .count();
        let mid = diverging_bar(Some(107_000_000), max, 20)
            .2
            .trim_end()
            .chars()
            .count();
        let small = diverging_bar(Some(3_000_000), max, 20)
            .2
            .trim_end()
            .chars()
            .count();
        assert_eq!(big, 20);
        assert!(mid < big && small < mid, "{big} {mid} {small}");
        assert!(
            mid >= small + 2,
            "107MB must be clearly longer than 3MB: {mid} vs {small}"
        );
    }

    #[test]
    fn noise_is_a_tick_not_a_bar() {
        let max = 10_000_000_000;
        let (_, _, r) = diverging_bar(Some(4_096), max, 20);
        assert_eq!(r.trim_end(), "▏", "a 4KB change is one tick");
        let (l, _, _) = diverging_bar(Some(-4_096), max, 20);
        assert_eq!(l.trim_start(), "▐", "and on the left it hugs the axis too");
        assert!(is_noise(Some(4_096)) && is_noise(Some(-4_096)));
        assert!(!is_noise(Some(0)), "no change is not noise, it is nothing");
        assert!(!is_noise(Some(NOISE_FLOOR)) && !is_noise(None));
    }

    #[test]
    fn spark_deltas_keep_the_biggest_move_per_bin_with_its_sign() {
        let s: Vec<Option<u64>> = vec![
            None,
            Some(10),
            Some(10),
            Some(19),
            Some(2),
            Some(2),
            Some(3),
        ];
        assert_eq!(
            deltas(&s),
            vec![None, Some(0), Some(9), Some(-17), Some(0), Some(1)]
        );
        assert_eq!(spark_deltas(&s, 3), vec![Some(0), Some(-17), Some(1)]);
        assert!(spark_deltas(&[], 5).is_empty());
    }

    #[test]
    fn flatness_means_nothing_moved() {
        assert!(is_flat(&[Some(5_186_904_064), Some(5_186_904_064)]));
        assert!(is_flat(&[None, None, Some(7)]));
        assert!(!is_flat(&[Some(100), Some(99)]));
        assert!(!is_flat(&[None, Some(0), Some(50)]));
    }

    #[test]
    fn trend_and_net_ignore_unobserved_buckets() {
        assert_eq!(trend(&[None, Some(1), Some(2)]), 1);
        assert_eq!(trend(&[Some(2), Some(1)]), -1);
        assert_eq!(trend(&[Some(3), None, Some(3)]), 0);
        assert_eq!(net_change(&[None, Some(10), Some(4)]), Some(-6));
        assert_eq!(net_change(&[None, Some(10)]), None);
    }

    #[test]
    fn truncate_middle_keeps_tail() {
        let long = "/Users/dev/src/some-really-long-project-name/node_modules";
        let t = truncate_middle(long, 20);
        assert!(t.len() <= long.len());
        assert!(t.ends_with("node_modules"));
        assert!(t.contains('…'));
    }

    #[test]
    fn truncate_middle_leaves_short_strings_alone() {
        assert_eq!(truncate_middle("short", 20), "short");
    }

    #[test]
    fn apply_sort_growth_puts_what_arrived_first_and_what_left_last() {
        let mut rows = vec![
            Row::leaf(0, "a".into(), 10, Some(5)),
            Row::leaf(0, "b".into(), 20, Some(-50)),
            Row::leaf(0, "c".into(), 30, Some(1)),
        ];
        apply_sort(&mut rows, Sort::Growth, false);
        assert_eq!(rows[0].label, "a", "the largest growth leads");
        assert_eq!(rows[1].label, "c");
        assert_eq!(
            rows[2].label, "b",
            "a big shrink sorts last, not first: it is not what grew"
        );
    }

    #[test]
    fn apply_sort_size_orders_by_bytes() {
        let mut rows = vec![
            Row::leaf(0, "a".into(), 10, None),
            Row::leaf(0, "b".into(), 300, None),
            Row::leaf(0, "c".into(), 20, None),
        ];
        apply_sort(&mut rows, Sort::Size, false);
        assert_eq!(rows[0].label, "b");
        assert_eq!(rows[1].label, "c");
        assert_eq!(rows[2].label, "a");
    }

    fn art(kind: ArtifactKind, path: &str, bytes: u64) -> swamp_core::report::ArtifactRow {
        swamp_core::report::ArtifactRow {
            kind,
            path: path.into(),
            bytes,
            mtime_max: 0,
            ecosystem: None,
            hardlinked: false,
            local_bytes: 0,
            track: None,
            growth_bytes: None,
            regrowth_count: 0,
            observed_at: 0,
            confidence: swamp_core::entities::Confidence::High,
            source: swamp_core::report::Source::new("t"),
            note: None,
            created_at: None,
            containers: Vec::new(),
            shared_with: Vec::new(),
            dangling: false,
        }
    }

    #[test]
    fn tree_rail_marks_last_sibling_with_an_elbow() {
        let report = Report {
            observed_at: 0,
            root: "/r".into(),
            projects: vec![swamp_core::report::ProjectRow {
                project_id: "p".into(),
                name: "proj".into(),
                remote: None,
                ecosystems: Vec::new(),
                worktrees: vec![
                    swamp_core::report::WorktreeRow {
                        worktree_id: "w1".into(),
                        path: "/r/proj".into(),
                        kind: swamp_core::report::WorktreeKind::Main,
                        artifacts: vec![
                            art(ArtifactKind::BuildOutput, "/r/proj/target", 10),
                            art(ArtifactKind::DependencyTree, "/r/proj/node_modules", 20),
                        ],
                        signals: vec![],
                        branch: None,
                        github: None,
                        merge_complete: None,
                        idle_secs: None,
                    },
                    swamp_core::report::WorktreeRow {
                        worktree_id: "w2".into(),
                        path: "/r/proj/.worktrees/x".into(),
                        kind: swamp_core::report::WorktreeKind::Linked,
                        artifacts: vec![],
                        signals: vec![],
                        branch: None,
                        github: None,
                        merge_complete: None,
                        idle_secs: None,
                    },
                ],
            }],
            unowned: vec![],
            reconciliation: swamp_core::report::Reconciliation {
                attributed: 0,
                unowned: 0,
                walked_total: 0,
                du_total: None,
                docker_attributed: 0,
                docker_unowned: 0,
            },
            notes: vec![],
            series_by_key: Default::default(),
            total_series: Vec::new(),
            series_window_secs: 0,
            summary: Default::default(),
            dirs_by_worktree: None,
            files_by_worktree: None,
            schedule_line: None,
            github_enrichment: None,
            nested_artifacts: Vec::new(),
        };
        let rows = tree_rows(
            &report,
            "proj",
            &Filter::default(),
            &Default::default(),
            &Default::default(),
        );
        // First worktree is not last -> ├─; second worktree is last -> └─.
        assert!(rows[0].rail.starts_with("├─"));
        assert!(rows[3].rail.starts_with("└─"));
        // Its two artifact children: first ├─, last └─, both under a │ rail
        // (first worktree is not the last sibling).
        assert!(rows[1].rail.starts_with("│  ├─"));
        assert!(rows[2].rail.starts_with("│  └─"));

        // Nested candidates must be selectable, while whole dependency groups
        // stay inspection-only. Inserting several children must not displace
        // the following worktree's rows.
        let tmp = tempfile::tempdir().unwrap();
        let target = std::fs::canonicalize(tmp.path()).unwrap().join("target");
        std::fs::create_dir_all(target.join("debug/incremental/crate-a")).unwrap();
        std::fs::create_dir_all(target.join("debug/deps")).unwrap();
        std::fs::write(target.join("debug/incremental/crate-a/state"), b"state").unwrap();
        let mut report = report;
        report.projects[0].worktrees[0].artifacts[0].path = target.clone();
        report.nested_artifacts =
            swamp_core::cargo_artifacts::inspect_target(&target, Some(&target)).units;
        let rows = builds_rows(&report, &Filter::default());
        let selected = UnitId::for_artifact(&target.join("debug/incremental/crate-a"));
        assert!(rows.iter().any(|r| r.unit == Some(selected.clone())));
        let deps = UnitId::for_artifact(&target.join("debug/deps"));
        assert!(!rows.iter().any(|r| r.unit == Some(deps.clone())));
    }

    #[test]
    fn docker_unowned_bytes_sums_only_docker_no_join() {
        let report = Report {
            observed_at: 0,
            root: "/r".into(),
            projects: vec![],
            unowned: vec![
                swamp_core::report::UnownedRow {
                    path_or_object: "img".into(),
                    bytes: 100,
                    reason: UnownedReason::DockerNoJoin,
                    docker_kind: None,
                    shared_bytes: None,
                    note: None,
                    created_at: None,
                    containers: Vec::new(),
                    shared_with: Vec::new(),
                    dangling: false,
                },
                swamp_core::report::UnownedRow {
                    path_or_object: "cache".into(),
                    bytes: 200,
                    reason: UnownedReason::SharedCache,
                    docker_kind: None,
                    shared_bytes: None,
                    note: None,
                    created_at: None,
                    containers: Vec::new(),
                    shared_with: Vec::new(),
                    dangling: false,
                },
            ],
            reconciliation: swamp_core::report::Reconciliation {
                attributed: 0,
                unowned: 0,
                walked_total: 0,
                du_total: None,
                docker_attributed: 0,
                docker_unowned: 0,
            },
            notes: vec![],
            series_by_key: Default::default(),
            total_series: Vec::new(),
            series_window_secs: 0,
            summary: Default::default(),
            dirs_by_worktree: None,
            files_by_worktree: None,
            schedule_line: None,
            github_enrichment: None,
            nested_artifacts: Vec::new(),
        };
        assert_eq!(docker_unowned_bytes(&report), 100);
    }
}
