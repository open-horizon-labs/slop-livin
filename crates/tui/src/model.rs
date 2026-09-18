//! Flattens a [`Report`] into displayable rows per view. Pure functions:
//! no I/O, no ratatui types, so this is unit-testable on its own.

use crate::filter::{self, Filter};
use crate::units::UnitId;
use slop_livin_core::report::{ArtifactKind, Report, UnownedReason};
use std::collections::BTreeMap;

/// Byte formatting is defined once, in core, so the TUI and the CLI can
/// never disagree about what "1.8GB" means (they did: one divided by 1024
/// under a decimal label while the other divided by 1000).
pub use slop_livin_core::render::human_bytes_pub as human_bytes;

pub use slop_livin_core::render::human_bytes_signed as human_signed_bytes;

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
    pub track: Option<slop_livin_core::ignore::TrackState>,
    /// Present for a worktree row: how many artifact children are hidden
    /// because the row is collapsed.
    pub collapsed_children: Option<usize>,
    pub expandable: bool,
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
            collapsed_children: None,
            expandable: false,
        }
    }
}

fn passes_filter(growth: Option<i64>, filter: &Filter) -> bool {
    filter::growth_passes(filter, growth)
}

/// Scale a growth delta to a bar of `+`/`-` characters, `width` wide,
/// relative to the largest |growth| in the visible set (never to size).
pub fn growth_bar(growth: Option<i64>, max_abs: i64, width: usize) -> String {
    let g = growth.unwrap_or(0);
    if max_abs <= 0 || g == 0 || width == 0 {
        return " ".repeat(width);
    }
    let filled = (((g.unsigned_abs() as f64 / max_abs as f64) * width as f64).round() as usize)
        .clamp(1, width);
    let ch = if g > 0 { '+' } else { '-' };
    let mut s = ch.to_string().repeat(filled);
    s.push_str(&" ".repeat(width - filled));
    s
}

pub fn max_abs_growth<'a>(rows: impl Iterator<Item = &'a Row>) -> i64 {
    rows.filter_map(|r| r.growth)
        .map(|g| g.abs())
        .max()
        .unwrap_or(0)
}

/// Applies `sort` to a flat (non-hierarchical) row list, most-growth or
/// most-bytes first. A stable sort keeps report order as the tiebreak.
pub fn apply_sort(rows: &mut [Row], sort: Sort) {
    match sort {
        Sort::None => {}
        Sort::Growth => {
            rows.sort_by(|a, b| {
                b.growth
                    .unwrap_or(0)
                    .abs()
                    .cmp(&a.growth.unwrap_or(0).abs())
            });
        }
        Sort::Size => rows.sort_by(|a, b| b.bytes.cmp(&a.bytes)),
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
        if !passes_filter(growth, filter) {
            continue;
        }
        out.push(Row {
            depth: 0,
            rail: String::new(),
            label: {
                let name = project_display_name(p);
                if p.worktrees.len() > 1 {
                    format!("{name}  · {} worktrees", p.worktrees.len())
                } else {
                    name
                }
            },
            bytes,
            growth,
            signals: Vec::new(),
            unit: None,
            kind: None,
            worktree: None,
            track: None,
            collapsed_children: None,
            expandable: true,
        });
    }
    out
}

/// Tree view for one project: checkout/worktree -> (folded) artifact
/// rows, built from the same [`slop_livin_core::tree::build_project_tree`]
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
    track: &std::collections::HashMap<std::path::PathBuf, slop_livin_core::ignore::TrackState>,
) -> Vec<Row> {
    let mut out = Vec::new();
    let Some(p) = report.projects.iter().find(|p| p.name == project_name) else {
        return out;
    };
    let tree = slop_livin_core::tree::build_project_tree(p, &report.root);
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
            linked: matches!(wt.kind, slop_livin_core::report::WorktreeKind::Linked),
            remote: p.remote.clone(),
            dirty: raw.0,
            unpushed: raw.1,
            locked: raw.2,
            merge_complete: source_wt
                .merge_complete
                .as_ref()
                .is_some_and(|m| m.verdict == slop_livin_core::github::TriState::Yes),
            pr: source_wt
                .github
                .as_ref()
                .and_then(|g| match &g.pull_request {
                    slop_livin_core::github::PrStatus::Some(pr) => {
                        Some(format!("PR #{} {:?}", pr.number, pr.state).to_lowercase())
                    }
                    _ => None,
                }),
        };
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
            collapsed_children: is_collapsed.then_some(wt.rows.len()),
            expandable: !wt.rows.is_empty(),
        });
        if is_collapsed {
            continue;
        }
        let child_prefix = if wt_last { "   " } else { "│  " };
        let visible: Vec<&slop_livin_core::tree::TreeRow> = wt
            .rows
            .iter()
            .filter(|row| {
                filter::kind_passes(filter, row.kind_label, row.kind.as_ref())
                    && passes_filter(row.growth_bytes, filter)
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
            let is_source = row.kind_label == "source";
            let source_key = format!("source:{}", source_wt.path.display());
            let source_collapsed = collapsed.contains(&source_key);
            let children: Vec<&slop_livin_core::report::DirRollup> = if is_source {
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
                    child.kind = Some(slop_livin_core::report::ArtifactKind::Unknown);
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

pub use slop_livin_core::render::project_display_name;

/// Top-level directories of a worktree's Source tree, biggest first.
/// Empty when the report was built without directory rollups.
fn source_children<'a>(
    report: &'a Report,
    worktree_id: &str,
) -> Vec<&'a slop_livin_core::report::DirRollup> {
    let Some(dirs) = report
        .dirs_by_worktree
        .as_ref()
        .and_then(|m| m.get(worktree_id))
    else {
        return Vec::new();
    };
    let mut top: Vec<&slop_livin_core::report::DirRollup> = dirs
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
            collapsed_children: None,
            expandable: false,
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
        out.push(Row::leaf(
            0,
            format!("unowned · {}", u.path_or_object),
            u.bytes,
            None,
        ));
    }
    out
}

/// Builds view: every `BuildOutput`/`Cache` row across the whole root,
/// same kind set as the CLI's `--view builds` (#33).
pub fn builds_rows(report: &Report) -> Vec<Row> {
    kind_filtered_rows(report, &[ArtifactKind::BuildOutput, ArtifactKind::Cache])
}

/// Deps view: every `DependencyTree` row across the whole root, same as
/// the CLI's `--view deps`.
pub fn deps_rows(report: &Report) -> Vec<Row> {
    kind_filtered_rows(report, &[ArtifactKind::DependencyTree])
}

fn kind_filtered_rows(report: &Report, kinds: &[ArtifactKind]) -> Vec<Row> {
    let mut out = Vec::new();
    for p in &report.projects {
        for wt in &p.worktrees {
            for a in &wt.artifacts {
                if !kinds.contains(&a.kind) {
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
    out
}

/// Unowned view.
pub fn unowned_rows(report: &Report) -> Vec<Row> {
    report
        .unowned
        .iter()
        .filter(|u| u.reason != UnownedReason::DockerNoJoin)
        .map(|u| {
            Row::leaf(
                0,
                format!("{:?} · {}", u.reason, u.path_or_object),
                u.bytes,
                None,
            )
        })
        .collect()
}

/// Worktree-level predicates against a report row's facts.
fn worktree_passes(filter: &Filter, wt: &slop_livin_core::report::WorktreeRow) -> bool {
    let merge_complete = wt
        .merge_complete
        .as_ref()
        .is_some_and(|m| m.verdict == slop_livin_core::github::TriState::Yes);
    let pr_state = wt.github.as_ref().and_then(|g| match &g.pull_request {
        slop_livin_core::github::PrStatus::Some(pr) => Some(&pr.state),
        _ => None,
    });
    filter::worktree_passes(filter, wt.idle_secs, merge_complete, pr_state)
}

/// Parses the rendered worktree signals back into facts for the mark
/// decision: (dirty, unpushed, locked). `None` = unknown.
trait RawWorktreeSignals {
    fn raw_signals(&self) -> (Option<bool>, Option<u32>, Option<bool>);
}
impl RawWorktreeSignals for slop_livin_core::report::WorktreeRow {
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
    fn growth_bar_scales_to_visible_max_not_size() {
        // Same growth, different max: bar length differs.
        let wide = growth_bar(Some(50), 100, 10);
        let narrow = growth_bar(Some(50), 50, 10);
        assert!(wide.trim_end().len() < narrow.trim_end().len());
    }

    #[test]
    fn growth_bar_uses_plus_for_growth_minus_for_shrink() {
        assert!(growth_bar(Some(10), 10, 4).starts_with('+'));
        assert!(growth_bar(Some(-10), 10, 4).starts_with('-'));
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
    fn growth_bar_empty_when_no_signal() {
        assert_eq!(growth_bar(None, 10, 4), "    ");
        assert_eq!(growth_bar(Some(0), 0, 4), "    ");
    }

    #[test]
    fn apply_sort_growth_orders_by_magnitude() {
        let mut rows = vec![
            Row::leaf(0, "a".into(), 10, Some(5)),
            Row::leaf(0, "b".into(), 20, Some(-50)),
            Row::leaf(0, "c".into(), 30, Some(1)),
        ];
        apply_sort(&mut rows, Sort::Growth);
        assert_eq!(rows[0].label, "b");
        assert_eq!(rows[1].label, "a");
        assert_eq!(rows[2].label, "c");
    }

    #[test]
    fn apply_sort_size_orders_by_bytes() {
        let mut rows = vec![
            Row::leaf(0, "a".into(), 10, None),
            Row::leaf(0, "b".into(), 300, None),
            Row::leaf(0, "c".into(), 20, None),
        ];
        apply_sort(&mut rows, Sort::Size);
        assert_eq!(rows[0].label, "b");
        assert_eq!(rows[1].label, "c");
        assert_eq!(rows[2].label, "a");
    }

    fn art(kind: ArtifactKind, path: &str, bytes: u64) -> slop_livin_core::report::ArtifactRow {
        slop_livin_core::report::ArtifactRow {
            kind,
            path: path.into(),
            bytes,
            local_bytes: 0,
            track: None,
            growth_bytes: None,
            regrowth_count: 0,
            observed_at: 0,
            confidence: slop_livin_core::entities::Confidence::High,
            source: slop_livin_core::report::Source::new("t"),
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
            projects: vec![slop_livin_core::report::ProjectRow {
                project_id: "p".into(),
                name: "proj".into(),
                remote: None,
                worktrees: vec![
                    slop_livin_core::report::WorktreeRow {
                        worktree_id: "w1".into(),
                        path: "/r/proj".into(),
                        kind: slop_livin_core::report::WorktreeKind::Main,
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
                    slop_livin_core::report::WorktreeRow {
                        worktree_id: "w2".into(),
                        path: "/r/proj/.worktrees/x".into(),
                        kind: slop_livin_core::report::WorktreeKind::Linked,
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
            reconciliation: slop_livin_core::report::Reconciliation {
                attributed: 0,
                unowned: 0,
                walked_total: 0,
                du_total: None,
                docker_attributed: 0,
                docker_unowned: 0,
            },
            notes: vec![],
            dirs_by_worktree: None,
            files_by_worktree: None,
            schedule_line: None,
            github_enrichment: None,
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
    }

    #[test]
    fn docker_unowned_bytes_sums_only_docker_no_join() {
        let report = Report {
            observed_at: 0,
            root: "/r".into(),
            projects: vec![],
            unowned: vec![
                slop_livin_core::report::UnownedRow {
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
                slop_livin_core::report::UnownedRow {
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
            reconciliation: slop_livin_core::report::Reconciliation {
                attributed: 0,
                unowned: 0,
                walked_total: 0,
                du_total: None,
                docker_attributed: 0,
                docker_unowned: 0,
            },
            notes: vec![],
            dirs_by_worktree: None,
            files_by_worktree: None,
            schedule_line: None,
            github_enrichment: None,
        };
        assert_eq!(docker_unowned_bytes(&report), 100);
    }
}
