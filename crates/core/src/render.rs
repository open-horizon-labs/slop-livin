//! Text rendering for [`crate::report::Report`].
//!
//! Signals only, never verdict vocabulary ("safe", "stale", "unused",
//! "abandoned", ...). Output is column-aligned ASCII that fits 100 cols
//! and needs no terminal color support.

use crate::report::{ArtifactKind, Report, UnownedReason, WorktreeKind};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

/// Default number of project rows shown in the overview before folding
/// the rest into a "… and N more" footer.
const DEFAULT_TOP_N: usize = 25;

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes}B")
    } else {
        format!("{value:.1}{}", UNITS[unit])
    }
}

fn human_signed_bytes(delta: i64) -> String {
    let sign = if delta > 0 {
        "+"
    } else if delta < 0 {
        "-"
    } else {
        ""
    };
    format!("{sign}{}", human_bytes(delta.unsigned_abs()))
}

fn kind_label(kind: &ArtifactKind) -> &'static str {
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

fn reason_label(reason: &UnownedReason) -> &'static str {
    match reason {
        UnownedReason::OutsideAnyCheckout => "outside-any-checkout",
        UnownedReason::OwnedByNothing => "owned-by-nothing",
        UnownedReason::InconclusiveEvidence => "inconclusive-evidence",
        UnownedReason::NoContainingRepo => "no-containing-repo",
        UnownedReason::SharedCache => "shared-cache",
        UnownedReason::PermissionDenied => "permission-denied",
        UnownedReason::DockerNoJoin => "docker-no-join",
    }
}

struct ProjectTotals {
    bytes: u64,
    growth: Option<i64>,
    worktrees: usize,
    top_kind: Option<(ArtifactKind, u64)>,
}

fn project_totals(project: &crate::report::ProjectRow) -> ProjectTotals {
    let mut bytes = 0u64;
    let mut growth: Option<i64> = None;
    let mut have_growth = false;
    let mut kind_bytes: BTreeMap<String, (ArtifactKind, u64)> = BTreeMap::new();
    for wt in &project.worktrees {
        for a in &wt.artifacts {
            bytes += a.bytes;
            if let Some(g) = a.growth_bytes {
                have_growth = true;
                growth = Some(growth.unwrap_or(0) + g);
            }
            let entry = kind_bytes
                .entry(kind_label(&a.kind).to_string())
                .or_insert((a.kind.clone(), 0));
            entry.1 += a.bytes;
        }
    }
    let top_kind = kind_bytes
        .into_values()
        .max_by_key(|(_, b)| *b)
        .filter(|(_, b)| *b > 0);
    ProjectTotals {
        bytes,
        growth: if have_growth { growth } else { None },
        worktrees: project.worktrees.len(),
        top_kind,
    }
}

fn header(report: &Report, verify_du: bool) -> String {
    let mut out = String::new();
    let worktree_count: usize = report.projects.iter().map(|p| p.worktrees.len()).sum();
    let _ = writeln!(out, "root: {}", report.root.display());
    let _ = write!(
        out,
        "observed_at={} projects={} worktrees={} attributed={} unowned={} walked={}",
        report.observed_at,
        report.projects.len(),
        worktree_count,
        human_bytes(report.reconciliation.attributed),
        human_bytes(report.reconciliation.unowned),
        human_bytes(report.reconciliation.walked_total),
    );
    if verify_du {
        let _ = write!(
            out,
            " du={}",
            report
                .reconciliation
                .du_total
                .map(human_bytes)
                .unwrap_or_else(|| "n/a".to_string())
        );
    }
    let _ = writeln!(out);
    out
}

/// The zero-flag, one-screen overview: header, then one line per project
/// sorted by growth desc then bytes desc, capped at `top_n` rows (0 means
/// no cap) with a "… and N more" footer.
pub fn render_overview(
    report: &Report,
    show_all: bool,
    verify_du: bool,
    show_docker: bool,
) -> String {
    let mut out = header(report, verify_du);
    let _ = writeln!(out);

    if report.projects.is_empty() {
        let _ = writeln!(out, "0 projects discovered under {}", report.root.display());
        return out;
    }

    let mut rows: Vec<(&crate::report::ProjectRow, ProjectTotals)> = report
        .projects
        .iter()
        .map(|p| (p, project_totals(p)))
        .collect();
    rows.sort_by(|a, b| {
        let ga = a.1.growth.unwrap_or(i64::MIN);
        let gb = b.1.growth.unwrap_or(i64::MIN);
        gb.cmp(&ga).then_with(|| b.1.bytes.cmp(&a.1.bytes))
    });

    let _ = writeln!(
        out,
        "{:<28} {:>10} {:>10} {:>4} {:<20}",
        "project", "bytes", "growth", "wts", "top kind"
    );
    let total = rows.len();
    let limit = if show_all { total } else { DEFAULT_TOP_N };
    for (project, totals) in rows.iter().take(limit) {
        let growth_str = totals
            .growth
            .map(human_signed_bytes)
            .unwrap_or_else(|| "—".to_string());
        let top_kind_str = totals
            .top_kind
            .as_ref()
            .map(|(k, b)| format!("{} ({})", kind_label(k), human_bytes(*b)))
            .unwrap_or_else(|| "0".to_string());
        let bytes_str = if totals.bytes == 0 {
            "0".to_string()
        } else {
            human_bytes(totals.bytes)
        };
        let _ = writeln!(
            out,
            "{:<28} {:>10} {:>10} {:>4} {:<20}",
            project.name, bytes_str, growth_str, totals.worktrees, top_kind_str
        );
    }
    if !show_all && total > limit {
        let _ = writeln!(out, "… and {} more (use --all)", total - limit);
    }
    let _ = writeln!(out);
    render_unowned_summary(report, &mut out, show_docker);
    out
}

/// `--docker` shows every unjoined Docker object individually; by default
/// they fold into one line per object kind (a real multi-project `~/src`
/// scan can have hundreds of unjoined build-cache entries, which used to
/// print one row each here).
fn render_unowned_summary(report: &Report, out: &mut String, show_docker: bool) {
    // Aggregate filesystem rows by their top-level directory *relative to
    // root* (never an absolute-path segment like `Users`, which every row
    // shares and which says nothing about where the bytes live) and by
    // reason; shared caches are listed separately. Docker objects are not
    // filesystem paths at all and are aggregated by kind instead. Never
    // per-file/per-object rows in the default summary.
    let mut by_dir: BTreeMap<String, u64> = BTreeMap::new();
    let mut by_reason: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut shared_caches_bytes = 0u64;
    let mut shared_caches_count = 0u64;
    let mut docker_by_kind: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut docker_rows: Vec<&crate::report::UnownedRow> = Vec::new();

    for row in &report.unowned {
        if row.reason == UnownedReason::DockerNoJoin {
            let kind = row
                .docker_kind
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let entry = docker_by_kind.entry(kind).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += row.bytes;
            docker_rows.push(row);
            continue;
        }
        if row.reason == UnownedReason::SharedCache {
            shared_caches_bytes += row.bytes;
            shared_caches_count += 1;
            continue;
        }
        let rel = Path::new(&row.path_or_object)
            .strip_prefix(&report.root)
            .unwrap_or_else(|_| Path::new(&row.path_or_object));
        let top_dir = rel
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .unwrap_or_else(|| row.path_or_object.clone());
        *by_dir.entry(top_dir).or_insert(0) += row.bytes;
        *by_reason.entry(reason_label(&row.reason)).or_insert(0) += row.bytes;
    }
    let _ = writeln!(out, "unowned by top-level dir:");
    if by_dir.is_empty() {
        let _ = writeln!(out, "  (none)");
    } else {
        for (dir, bytes) in &by_dir {
            let _ = writeln!(out, "  {:<30} {:>10}", dir, human_bytes(*bytes));
        }
    }
    let _ = writeln!(out, "unowned by reason:");
    if by_reason.is_empty() {
        let _ = writeln!(out, "  (none)");
    } else {
        for (reason, bytes) in &by_reason {
            let _ = writeln!(out, "  {:<30} {:>10}", reason, human_bytes(*bytes));
        }
    }
    if shared_caches_count > 0 {
        let _ = writeln!(
            out,
            "shared caches: {} ({} items)",
            human_bytes(shared_caches_bytes),
            shared_caches_count
        );
    }
    if !docker_by_kind.is_empty() {
        let _ = writeln!(out, "docker unowned:");
        for (kind, (count, bytes)) in &docker_by_kind {
            let _ = writeln!(
                out,
                "  {:<30} {:>10} ({} items)",
                kind,
                human_bytes(*bytes),
                count
            );
        }
        if !show_docker {
            let _ = writeln!(out, "  (use --docker to list each object)");
        }
    }
    if show_docker {
        for row in docker_rows {
            let _ = writeln!(
                out,
                "  {:<40} {:>10}",
                row.path_or_object,
                human_bytes(row.bytes)
            );
        }
    }
}

fn worktree_kind_label(kind: &WorktreeKind) -> &'static str {
    match kind {
        WorktreeKind::Main => "main",
        WorktreeKind::Linked => "linked",
        WorktreeKind::Clone => "clone",
    }
}

/// `--project <name>` drill: worktree → kind → path → bytes → growth →
/// regrowth → signals.
///
/// Worktree identity is shown as its path relative to the report root
/// plus a short 8-char id (the full 64-hex `worktree_id` is noise on a
/// terminal screen and never needed to tell rows apart here); a
/// duplicate relative path -- two truly distinct identities that happen
/// to render the same, which the 8-char id then disambiguates -- keeps
/// the full path as a fallback suffix.
pub fn render_project(report: &Report, name: &str) -> Option<String> {
    let project = report.projects.iter().find(|p| p.name == name)?;
    let mut out = String::new();
    let _ = writeln!(out, "project: {}", project.name);
    if project.worktrees.is_empty() {
        let _ = writeln!(out, "  (no worktrees)");
        return Some(out);
    }
    for wt in &project.worktrees {
        let kind = worktree_kind_label(&wt.kind);
        let rel = wt
            .path
            .strip_prefix(&report.root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| wt.path.display().to_string());
        let short_id = &wt.worktree_id[..wt.worktree_id.len().min(8)];
        let _ = writeln!(out, "worktree: {rel} ({short_id}) [{kind}]");
        if wt.artifacts.is_empty() {
            let _ = writeln!(out, "  0");
        }
        for a in &wt.artifacts {
            let growth_str = a
                .growth_bytes
                .map(human_signed_bytes)
                .unwrap_or_else(|| "—".to_string());
            let _ = writeln!(
                out,
                "  {:<14} {:<40} {:>10} {:>10} regrowth={}",
                kind_label(&a.kind),
                a.path.display(),
                human_bytes(a.bytes),
                growth_str,
                a.regrowth_count,
            );
        }
        if !wt.signals.is_empty() {
            let signals = wt
                .signals
                .iter()
                .map(|s| format!("{}: {}", s.name, s.value))
                .collect::<Vec<_>>()
                .join(" · ");
            let _ = writeln!(out, "  signals: {signals}");
        }
    }
    Some(out)
}

/// `--kinds`: bytes and count per artifact kind across the root.
pub fn render_kinds(report: &Report) -> String {
    let mut out = String::new();
    let mut agg: BTreeMap<&'static str, (u64, u64)> = BTreeMap::new();
    for project in &report.projects {
        for wt in &project.worktrees {
            for a in &wt.artifacts {
                let entry = agg.entry(kind_label(&a.kind)).or_insert((0, 0));
                entry.0 += a.bytes;
                entry.1 += 1;
            }
        }
    }
    let _ = writeln!(out, "{:<16} {:>12} {:>8}", "kind", "bytes", "count");
    if agg.is_empty() {
        let _ = writeln!(out, "0");
        return out;
    }
    let mut rows: Vec<(&str, (u64, u64))> = agg.into_iter().collect();
    rows.sort_by(|a, b| b.1.0.cmp(&a.1.0));
    for (kind, (bytes, count)) in rows {
        let _ = writeln!(out, "{:<16} {:>12} {:>8}", kind, human_bytes(bytes), count);
    }
    out
}

/// Legacy flat renderer kept for the golden test's exact-format
/// expectations (R2-R5 fixture output); the CLI's default surface is
/// [`render_overview`].
pub fn render_text(report: &Report) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "root: {}", report.root.display());
    let _ = writeln!(out, "observed_at: {}", report.observed_at);
    let _ = writeln!(out);
    if report.projects.is_empty() {
        let _ = writeln!(out, "0 projects discovered under {}", report.root.display());
        return out;
    }
    let _ = writeln!(
        out,
        "{:<12} {:<10} {:<10} {:<8} {:<40} {:>12} {:>12} {:>8}",
        "project", "worktree", "kind", "artifact", "path", "bytes", "growth", "regrowth"
    );
    for project in &report.projects {
        for worktree in &project.worktrees {
            let kind = worktree_kind_label(&worktree.kind);
            for artifact in &worktree.artifacts {
                let _ = writeln!(
                    out,
                    "{:<12} {:<10} {:<10} {:<8} {:<40} {:>12} {:>12} {:>8}",
                    project.name,
                    &worktree.worktree_id[..worktree.worktree_id.len().min(10)],
                    kind,
                    format!("{:?}", artifact.kind),
                    artifact.path.display(),
                    artifact.bytes,
                    artifact
                        .growth_bytes
                        .map(|g| g.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    artifact.regrowth_count,
                );
            }
            if !worktree.signals.is_empty() {
                let signals = worktree
                    .signals
                    .iter()
                    .map(|s| format!("{}={}", s.name, s.value))
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(out, "  signals[{}]: {}", worktree.worktree_id, signals);
            }
        }
    }
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{:<40} {:>12} {:<20}",
        "unowned path/object", "bytes", "reason"
    );
    for row in &report.unowned {
        let _ = writeln!(
            out,
            "{:<40} {:>12} {:<20?}",
            row.path_or_object, row.bytes, row.reason
        );
    }
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "reconciliation: attributed={} unowned={} walked_total={} du_total={}",
        report.reconciliation.attributed,
        report.reconciliation.unowned,
        report.reconciliation.walked_total,
        report
            .reconciliation
            .du_total
            .map(|v| v.to_string())
            .unwrap_or_else(|| "n/a".to_string()),
    );
    out
}
