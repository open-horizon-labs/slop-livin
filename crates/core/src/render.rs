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

/// Decimal (SI, ÷1000) so a "47.9GB" this renderer prints means the same
/// 47.9 * 10^9 a raw byte count means everywhere else (e.g. `walked_total`
/// printed verbatim in the `observe` log line, or read out of `report
/// --json`). Before this fix the divisor was 1024 (binary/GiB) under a
/// decimal ("GB") label, so the *same* observation could read "51.3GB"
/// from one surface (the raw integer) and "47.9GB" from this one -- a
/// 1024-vs-1000 unit mismatch masquerading as stale/re-read data.
/// A project's display name: `owner/repo` when its remote names one, so
/// two clones of different repos sharing a basename are told apart on
/// sight. Shared by the TUI, CLI and MCP.
pub fn project_display_name(p: &crate::report::ProjectRow) -> String {
    let owner_repo = p.remote.as_deref().and_then(|r| {
        let parts: Vec<&str> = r.trim_end_matches('/').split('/').collect();
        (parts.len() >= 3).then(|| format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1]))
    });
    match owner_repo {
        Some(or) if or.to_lowercase().ends_with(&p.name.to_lowercase()) => or,
        _ => p.name.clone(),
    }
}

/// The one byte formatter in this product (decimal, SI-labelled). The TUI
/// re-exports it; a second implementation is a defect (source audit).
pub fn human_bytes_pub(bytes: u64) -> String {
    human_bytes(bytes)
}

/// Signed human units for growth/delta figures: `+1.2GB`, `-300.0MB`, `0B`.
pub fn human_bytes_signed(delta: i64) -> String {
    if delta == 0 {
        return "0B".into();
    }
    let sign = if delta < 0 { "-" } else { "+" };
    format!("{sign}{}", human_bytes(delta.unsigned_abs()))
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
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
        ArtifactKind::Ignored => "ignored",
        ArtifactKind::Untracked => "untracked",
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
    if let Some(line) = &report.schedule_line {
        let _ = writeln!(out, "{line}");
    }
    out
}

/// The zero-flag, one-screen overview: header, then one line per project
/// sorted by growth desc then bytes desc, capped at `top_n` rows (0 means
/// no cap) with a "… and N more" footer.
/// How `render_overview` orders project rows. `Growth` is the default the
/// tool always had (growth desc, then bytes); the rest mirror the TUI's
/// `g/s/n/t/a` keys and clean-dev-dirs' `--sort size|age|name|type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverviewSort {
    #[default]
    Growth,
    Size,
    Name,
    Type,
    Age,
}

impl std::str::FromStr for OverviewSort {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "growth" => OverviewSort::Growth,
            "size" | "bytes" => OverviewSort::Size,
            "name" => OverviewSort::Name,
            "type" => OverviewSort::Type,
            "age" => OverviewSort::Age,
            other => {
                return Err(format!(
                    "unknown sort {other:?}; use growth|size|name|type|age"
                ));
            }
        })
    }
}

pub fn render_overview(
    report: &Report,
    show_all: bool,
    verify_du: bool,
    show_docker: bool,
) -> String {
    render_overview_sorted(
        report,
        show_all,
        verify_du,
        show_docker,
        OverviewSort::Growth,
        false,
    )
}

/// `--view types`: one line per ecosystem from `Report.summary`.
pub fn render_types(report: &Report) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<6} {:<14} {:>9} {:>10} {:>12} {:>10}",
        "type", "name", "projects", "artifacts", "bytes", "growth"
    );
    let mut rows: Vec<_> = report.summary.by_type.iter().collect();
    rows.sort_by(|a, b| b.1.bytes.cmp(&a.1.bytes));
    if rows.is_empty() {
        let _ = writeln!(out, "0");
        return out;
    }
    for (tag, t) in rows {
        let _ = writeln!(
            out,
            "{:<6} {:<14} {:>9} {:>10} {:>12} {:>10}",
            tag,
            t.name,
            t.projects,
            t.artifacts,
            human_bytes(t.bytes),
            t.growth_bytes
                .map(human_signed_bytes)
                .unwrap_or_else(|| "—".to_string())
        );
    }
    out
}

pub fn render_overview_sorted(
    report: &Report,
    show_all: bool,
    verify_du: bool,
    show_docker: bool,
    sort: OverviewSort,
    reverse: bool,
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
    let type_rank = |p: &crate::report::ProjectRow| {
        p.ecosystems
            .first()
            .and_then(|t| crate::ecosystem::ECOSYSTEMS.iter().position(|e| e.tag == t))
            .unwrap_or(usize::MAX)
    };
    let age_key = |p: &crate::report::ProjectRow| {
        let m = p
            .worktrees
            .iter()
            .flat_map(|w| w.artifacts.iter())
            .map(|a| a.mtime_max)
            .max()
            .unwrap_or(0);
        if m == 0 { u64::MAX } else { m }
    };
    match sort {
        OverviewSort::Growth => rows.sort_by(|a, b| {
            let ga = a.1.growth.unwrap_or(i64::MIN);
            let gb = b.1.growth.unwrap_or(i64::MIN);
            gb.cmp(&ga).then_with(|| b.1.bytes.cmp(&a.1.bytes))
        }),
        OverviewSort::Size => rows.sort_by(|a, b| b.1.bytes.cmp(&a.1.bytes)),
        OverviewSort::Name => {
            rows.sort_by(|a, b| a.0.name.to_lowercase().cmp(&b.0.name.to_lowercase()))
        }
        OverviewSort::Type => rows.sort_by(|a, b| {
            type_rank(a.0)
                .cmp(&type_rank(b.0))
                .then_with(|| b.1.bytes.cmp(&a.1.bytes))
        }),
        OverviewSort::Age => rows.sort_by(|a, b| age_key(a.0).cmp(&age_key(b.0))),
    }
    if reverse {
        rows.reverse();
    }

    let _ = writeln!(
        out,
        "{:<10} {:<28} {:>10} {:>10} {:>4} {:<20}",
        "type", "project", "bytes", "growth", "wts", "top kind"
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
            "{:<10} {:<28} {:>10} {:>10} {:>4} {:<20}",
            crate::ecosystem::tags(&project.ecosystems),
            project.name,
            bytes_str,
            growth_str,
            totals.worktrees,
            top_kind_str
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

/// `--view worktrees [--filter '...']`: one line per matching worktree,
/// with the literal human command to remove it. Never executed by this
/// tool -- the text is printed for a person to run themselves.
pub fn render_worktrees(report: &Report, filter: &crate::filter::Filter) -> String {
    use crate::github::{GithubFacts, MergeComplete, MergedStatus, PrStatus, TriState};
    let mut out = String::new();
    let mut shown = 0usize;
    let empty_pr = PrStatus::Unknown;
    for project in &report.projects {
        for wt in &project.worktrees {
            let (pr, verdict, merge_complete_terms, merged) = match (&wt.github, &wt.merge_complete)
            {
                (
                    Some(GithubFacts {
                        pull_request,
                        merged,
                        ..
                    }),
                    Some(MergeComplete { verdict, terms }),
                ) => (pull_request, *verdict, Some(terms.clone()), merged),
                (
                    Some(GithubFacts {
                        pull_request,
                        merged,
                        ..
                    }),
                    None,
                ) => (pull_request, TriState::Unknown, None, merged),
                (None, _) => (&empty_pr, TriState::Unknown, None, &MergedStatus::Unknown),
            };

            let facts = crate::filter::WorktreeFacts {
                merge_complete: verdict == TriState::Yes,
                idle_secs: wt.idle_secs,
                pr,
                merged,
            };
            if !filter.matches_worktree(project, wt, &facts) {
                continue;
            }
            shown += 1;

            let branch = wt
                .branch
                .clone()
                .unwrap_or_else(|| "(detached)".to_string());
            let idle_str = wt
                .idle_secs
                .map(|s| format!("idle {}", human_duration(s)))
                .unwrap_or_else(|| "idle unknown".to_string());
            let verdict_str = match verdict {
                TriState::Yes => "yes",
                TriState::No => "no",
                TriState::Unknown => "unknown",
            };
            let mc_str = match merge_complete_terms {
                Some(terms) => format!("merge-complete: {verdict_str} ({})", terms.join(", ")),
                None => "merge-complete: unknown (no GitHub remote)".to_string(),
            };
            let pr_str = render_pr(pr);

            let _ = writeln!(
                out,
                "{}  {}  branch={}  {}  {}  {}",
                project.name,
                wt.path.display(),
                branch,
                idle_str,
                mc_str,
                pr_str,
            );
            match wt.kind {
                WorktreeKind::Linked => {
                    let _ = writeln!(out, "  git worktree remove {}", wt.path.display());
                }
                WorktreeKind::Main | WorktreeKind::Clone => {
                    let _ = writeln!(out, "  main checkout -- not removable as a worktree");
                }
            }
        }
    }
    if shown == 0 {
        let _ = writeln!(out, "0 worktrees match");
    }
    out
}

/// Bare duration ("3d", "4h", "12m", "45s"), no prefix.
fn human_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

fn render_pr(pr: &crate::github::PrStatus) -> String {
    use crate::github::{PrState, PrStatus, ReviewDecision};
    match pr {
        PrStatus::None => "no PR".to_string(),
        PrStatus::Unknown => "PR unknown".to_string(),
        PrStatus::Some(pr) => {
            let state = match pr.state {
                PrState::Open => "open",
                PrState::Closed => "closed",
                PrState::Merged => "merged",
            };
            let decision = match pr.review_decision {
                ReviewDecision::Approved => Some("approved"),
                ReviewDecision::ChangesRequested => Some("changes requested"),
                ReviewDecision::ReviewRequired => Some("review required"),
                ReviewDecision::None | ReviewDecision::Unknown => None,
            };
            match decision {
                Some(d) => format!("PR #{} {state} ({d})", pr.number),
                None => format!("PR #{} {state}", pr.number),
            }
        }
    }
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

/// `--project <name>` (or `--project <name> --view worktrees`, the
/// default view): the project's tree, checkouts/worktrees down to
/// (folded) artifact rows, per #33. Replaces the old flat
/// `render_project` drill; paths on every row are relative (never
/// absolute, which the flat drill used to leak), and a signal is
/// printed as its value only, never `name: name: value`.
pub fn render_project_tree(report: &Report, name: &str) -> Option<String> {
    let project = report.projects.iter().find(|p| p.name == name)?;
    let tree = crate::tree::build_project_tree(project, &report.root);
    let mut out = String::new();
    let growth_str = tree
        .growth_bytes
        .map(|g| {
            // `human_signed_bytes` already carries the sign. Stripping a
            // leading `+` and then hardcoding one printed `+-18.9MB` for
            // a shrink; only an exact zero needs a sign added.
            let signed = human_signed_bytes(g);
            let signed = if g == 0 { format!("+{signed}") } else { signed };
            format!(" ({signed}/24h)")
        })
        .unwrap_or_default();
    let _ = writeln!(
        out,
        "{}  {}{}",
        tree.name,
        human_bytes(tree.bytes),
        growth_str
    );
    if tree.worktrees.is_empty() {
        let _ = writeln!(out, "  (no worktrees)");
        return Some(out);
    }
    let last_idx = tree.worktrees.len() - 1;
    for (i, wt) in tree.worktrees.iter().enumerate() {
        let branch = if i == last_idx { "└─" } else { "├─" };
        let child_prefix = if i == last_idx { "   " } else { "│  " };
        let kind = worktree_kind_label(&wt.kind);
        let growth = wt
            .growth_bytes
            .map(human_signed_bytes)
            .unwrap_or_else(|| "—".to_string());
        let signals = if wt.signals.is_empty() {
            String::new()
        } else {
            format!(
                "   {}",
                wt.signals
                    .iter()
                    .map(|s| s.value.clone())
                    .collect::<Vec<_>>()
                    .join(" · ")
            )
        };
        let _ = writeln!(
            out,
            "{branch} {:<8} {:<28} {:>10}  ({growth})   {signals}",
            kind,
            wt.rel_path,
            human_bytes(wt.bytes),
        );
        let row_last = wt.rows.len().checked_sub(1);
        for (j, row) in wt.rows.iter().enumerate() {
            let row_branch = if Some(j) == row_last {
                "└─"
            } else {
                "├─"
            };
            let growth = row
                .growth_bytes
                .map(human_signed_bytes)
                .unwrap_or_else(|| "—".to_string());
            let label = if row.folded_count > 1 {
                format!("{} (x{})", row.rel_path, row.folded_count)
            } else {
                row.rel_path.clone()
            };
            let _ = writeln!(
                out,
                "{child_prefix}{row_branch} {:<10} {:<30} {:>10}  ({growth})",
                row.kind_label,
                label,
                human_bytes(row.bytes),
            );
        }
    }
    Some(out)
}

/// `--view builds`: every `BuildOutput`/`Cache` row across the project
/// (or the whole root when `only_project` is `None`), sorted by bytes
/// desc.
pub fn render_view_builds(report: &Report, only_project: Option<&str>) -> String {
    render_kind_view(
        report,
        only_project,
        &[ArtifactKind::BuildOutput, ArtifactKind::Cache],
    )
}

/// `--view deps`: every `DependencyTree` row across the project (or
/// root), sorted by bytes desc. Shared dependency caches that touch this
/// project surface as ordinary `DependencyTree` rows already joined at
/// discovery time; this view does not re-derive that join.
pub fn render_view_deps(report: &Report, only_project: Option<&str>) -> String {
    render_kind_view(report, only_project, &[ArtifactKind::DependencyTree])
}

/// rust view: nested Cargo units inside already-accounted target rows.
/// Aggregate rows show logical bytes while leaves show physically charged
/// bytes, making hardlink and residual limits visible.
pub fn render_view_rust(report: &Report, only_project: Option<&str>) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<18} {:<20} {:<10} {:>10} {:>10} {:>10}  path / evidence",
        "project", "role", "profile", "allocated", "charged", "growth"
    );
    let mut rows = Vec::new();
    for unit in &report.nested_artifacts {
        let project = report
            .projects
            .iter()
            .find(|p| p.worktrees.iter().any(|w| unit.path.starts_with(&w.path)))
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "unknown".into());
        if only_project.is_some_and(|wanted| wanted != project) {
            continue;
        }
        let profile = unit.variant.profile.clone().unwrap_or_else(|| "?".into());
        let evidence = unit
            .producer_evidence
            .first()
            .map(|e| e.source.as_str())
            .unwrap_or("unknown");
        rows.push((
            project,
            unit.role.label(),
            profile,
            unit.bytes,
            unit.physical_total,
            unit,
            evidence,
        ));
    }
    rows.sort_by(|a, b| {
        b.3.cmp(&a.3)
            .then_with(|| a.5.relative_path.cmp(&b.5.relative_path))
    });
    if rows.is_empty() {
        let _ = writeln!(out, "0 (no Cargo target rows or no supported nested facts)");
        return out;
    }
    for (project, role, profile, bytes, charged, unit, evidence) in rows {
        let unknown = if unit.variant.unknowns.is_empty() {
            String::new()
        } else {
            format!("; unknown: {}", unit.variant.unknowns.join(", "))
        };
        let _ = writeln!(
            out,
            "{:<18} {:<20} {:<10} {:>10} {:>10} {:>10}  {} [{}{}]",
            project,
            role,
            profile,
            human_bytes(bytes),
            if matches!(
                unit.membership,
                crate::artifact::Membership::Unknown | crate::artifact::Membership::SharedHardlink
            ) {
                "—".into()
            } else {
                human_bytes(charged)
            },
            unit.growth_bytes
                .map(human_bytes_signed)
                .unwrap_or_else(|| "—".into()),
            unit.relative_path,
            evidence,
            unknown
        );
    }
    let _ = writeln!(
        out,
        "Parent rows include their children: do not sum them. Charged bytes are inode-deduplicated allocation, not reclaimable space. debug/release name output directories, not unique dev/test/bench configurations."
    );
    out
}

fn render_kind_view(report: &Report, only_project: Option<&str>, kinds: &[ArtifactKind]) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<20} {:<10} {:<40} {:>10} {:>10}",
        "project", "kind", "path", "bytes", "growth"
    );
    let mut rows: Vec<(String, &'static str, String, u64, Option<i64>)> = Vec::new();
    for project in &report.projects {
        if let Some(name) = only_project
            && project.name != name
        {
            continue;
        }
        for wt in &project.worktrees {
            for a in &wt.artifacts {
                if !kinds.contains(&a.kind) {
                    continue;
                }
                let rel = a
                    .path
                    .strip_prefix(&wt.path)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| a.path.display().to_string());
                rows.push((
                    project.name.clone(),
                    kind_label(&a.kind),
                    rel,
                    a.bytes,
                    a.growth_bytes,
                ));
            }
        }
    }
    rows.sort_by(|a, b| b.3.cmp(&a.3));
    if rows.is_empty() {
        let _ = writeln!(out, "0");
        return out;
    }
    for (project, kind, path, bytes, growth) in rows {
        let growth_str = growth
            .map(human_signed_bytes)
            .unwrap_or_else(|| "—".to_string());
        let _ = writeln!(
            out,
            "{:<20} {:<10} {:<40} {:>10} {:>10}",
            project,
            kind,
            path,
            human_bytes(bytes),
            growth_str
        );
    }
    out
}

/// `--view docker`: every Docker object joined to the project (or, at
/// root, every joined Docker object across every project) plus, when
/// `only_project` is set, unowned Docker objects whose name family
/// resembles the project -- shown clearly labelled as unowned and never
/// attributed, per #33/#19 (no verdict words, name similarity is never
/// evidence). Sorted by bytes (unique bytes) desc.
pub fn render_view_docker(report: &Report, only_project: Option<&str>) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<24} {:<20} {:<14} {:>10} {:>10}  detail",
        "project", "object", "kind", "bytes", "shared"
    );

    /// One `--view docker` row's detail column, assembled from
    /// created_at / shared_with / containers / dangling / note so every
    /// object -- joined or unowned -- shows the same fields (#33).
    fn detail_string(
        created_at: &Option<String>,
        shared_with: &[String],
        containers: &[String],
        dangling: bool,
        note: &Option<String>,
    ) -> String {
        let mut bits: Vec<String> = Vec::new();
        if let Some(c) = created_at {
            bits.push(format!("created {c}"));
        }
        if dangling {
            bits.push("dangling".to_string());
        }
        if !shared_with.is_empty() {
            bits.push(format!("shared_with={}", shared_with.join(",")));
        }
        if containers.is_empty() {
            bits.push("no containers reference it".to_string());
        } else {
            bits.push(format!("containers={}", containers.join("; ")));
        }
        if let Some(n) = note {
            bits.push(n.clone());
        }
        bits.join(" · ")
    }

    struct Row {
        project: String,
        object: String,
        kind: String,
        bytes: u64,
        shared_bytes: u64,
        detail: String,
    }
    let mut rows: Vec<Row> = Vec::new();
    for project in &report.projects {
        if let Some(name) = only_project
            && project.name != name
        {
            continue;
        }
        for wt in &project.worktrees {
            for a in &wt.artifacts {
                if !matches!(
                    a.kind,
                    ArtifactKind::DockerImage
                        | ArtifactKind::DockerBuildCache
                        | ArtifactKind::DockerVolume
                ) {
                    continue;
                }
                rows.push(Row {
                    project: project.name.clone(),
                    object: a.path.display().to_string(),
                    kind: kind_label(&a.kind).to_string(),
                    bytes: a.bytes,
                    shared_bytes: 0,
                    detail: detail_string(
                        &a.created_at,
                        &a.shared_with,
                        &a.containers,
                        a.dangling,
                        &a.note,
                    ),
                });
            }
        }
    }
    for row in &report.unowned {
        if row.reason != UnownedReason::DockerNoJoin {
            continue;
        }
        // At root (no project filter): list every unowned object too, so
        // `--view docker` at root is a complete listing per the issue.
        // With `--project`: only include a candidate whose reference
        // string contains the project name -- shown as unowned, never
        // attributed.
        let project_label = match only_project {
            None => String::new(),
            Some(name) => {
                if row
                    .path_or_object
                    .to_lowercase()
                    .contains(&name.to_lowercase())
                {
                    format!("{name} (unowned, name-alike)")
                } else {
                    continue;
                }
            }
        };
        rows.push(Row {
            project: project_label,
            object: row.path_or_object.clone(),
            kind: row.docker_kind.as_deref().unwrap_or("unknown").to_string(),
            bytes: row.bytes,
            shared_bytes: row.shared_bytes.unwrap_or(0),
            detail: detail_string(
                &row.created_at,
                &row.shared_with,
                &row.containers,
                row.dangling,
                &row.note,
            ),
        });
    }
    rows.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    if rows.is_empty() {
        let _ = writeln!(out, "0");
        return out;
    }
    for row in rows {
        let _ = writeln!(
            out,
            "{:<24} {:<20} {:<14} {:>10} {:>10}  {}",
            row.project,
            row.object,
            row.kind,
            human_bytes(row.bytes),
            if row.shared_bytes > 0 {
                human_bytes(row.shared_bytes)
            } else {
                "—".to_string()
            },
            row.detail,
        );
    }
    out
}

/// `--view reconciliation`: attributed / unowned / walked / du (when
/// computed) / docker in one line, exactly the "one command" the
/// maintainer rule comment asks for instead of hand-assembled jq over
/// separate fields.
pub fn render_view_reconciliation(report: &Report) -> String {
    let mut out = String::new();
    let r = &report.reconciliation;
    let _ = writeln!(
        out,
        "attributed={} unowned={} walked={} du={} docker_attributed={} docker_unowned={}",
        human_bytes(r.attributed),
        human_bytes(r.unowned),
        human_bytes(r.walked_total),
        r.du_total
            .map(human_bytes)
            .unwrap_or_else(|| "n/a".to_string()),
        human_bytes(r.docker_attributed),
        human_bytes(r.docker_unowned),
    );
    out
}

/// `--worktree <path>`: signals for one worktree, matched by exact path
/// or by its relative path under the report root, since a project's
/// `WorktreeRow.path` is stored absolute. One command, no jq over the
/// whole report needed to answer "signals for this worktree".
pub fn render_worktree_signals(report: &Report, path: &Path) -> Option<String> {
    let worktree = report.projects.iter().find_map(|p| {
        p.worktrees.iter().find(|w| {
            w.path == path
                || w.path
                    .strip_prefix(&report.root)
                    .map(|rel| rel == path)
                    .unwrap_or(false)
        })
    })?;
    let mut out = String::new();
    let _ = writeln!(out, "worktree: {}", worktree.path.display());
    if worktree.signals.is_empty() {
        let _ = writeln!(out, "  (no signals)");
    } else {
        for s in &worktree.signals {
            let _ = writeln!(out, "  {}: {}", s.name, s.value);
        }
    }
    Some(out)
}

/// `--view unowned` at root: the same aggregation `render_overview`
/// prints below the project table, standalone so it is one command
/// rather than a post-processed slice of the overview.
pub fn render_view_unowned(report: &Report) -> String {
    let mut out = String::new();
    render_unowned_summary(report, &mut out, true);
    out
}
