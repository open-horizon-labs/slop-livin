//! Report contract: the shape every later slice fills in.
//!
//! This module defines types and rendering only. Discovery, attribution,
//! growth, and Docker joins are later slices (R2-R5); `report()` here is a
//! stub entry point that returns an empty report so the golden test can
//! exercise the real contract shape before any discovery logic exists.

use crate::entities::{Confidence, id_for};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Where a fact came from (a tool invocation, a filesystem walk, ...).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Source {
    pub tool: String,
}

impl Source {
    pub fn new(tool: impl Into<String>) -> Self {
        Self { tool: tool.into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ArtifactKind {
    BuildOutput,
    DependencyTree,
    Git,
    /// A rebuildable tool/download cache found *inside* a checkout or
    /// worktree (e.g. `.cache`, `.parcel-cache`). A cache of the same
    /// shape found *outside* every checkout is not an artifact row at
    /// all; it becomes an `UnownedRow` with reason `SharedCache` instead.
    Cache,
    /// Bytes under a worktree that are not inside any classified
    /// artifact directory: source files, VCS-tracked content, and small
    /// housekeeping (a linked worktree's `.git` file, etc). Exactly one
    /// `Source` row per worktree.
    Source,
    DockerImage,
    DockerCache,
    DockerVolume,
    Loose,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorktreeKind {
    Main,
    Linked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum UnownedReason {
    OutsideAnyCheckout,
    OwnedByNothing,
    InconclusiveEvidence,
    /// Outside every discovered checkout/worktree and not a recognized
    /// shared cache: nothing claims this path.
    NoContainingRepo,
    /// Outside every discovered checkout/worktree, but its basename/kind
    /// is a cache shape shared across projects (`.cache`,
    /// `.cargo-registry`, `.npm`, `.pnpm-store`, a top-level `.gradle`).
    /// Counted exactly once here, never duplicated into any project.
    SharedCache,
    /// The walk could not read this path (permissions). Bytes are
    /// recorded as 0; the row exists so the count is visible instead of
    /// silently dropped.
    PermissionDenied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRow {
    pub kind: ArtifactKind,
    pub path: PathBuf,
    pub bytes: u64,
    pub growth_bytes: Option<i64>,
    pub regrowth_count: u32,
    pub observed_at: u64,
    pub confidence: Confidence,
    pub source: Source,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeRow {
    pub worktree_id: String,
    pub path: PathBuf,
    pub kind: WorktreeKind,
    pub artifacts: Vec<ArtifactRow>,
    pub signals: Vec<Signal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRow {
    pub project_id: String,
    pub name: String,
    pub worktrees: Vec<WorktreeRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnownedRow {
    pub path_or_object: String,
    pub bytes: u64,
    pub reason: UnownedReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reconciliation {
    pub attributed: u64,
    pub unowned: u64,
    pub walked_total: u64,
    pub du_total: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub observed_at: u64,
    pub root: PathBuf,
    pub projects: Vec<ProjectRow>,
    pub unowned: Vec<UnownedRow>,
    pub reconciliation: Reconciliation,
}

/// R2 discovers projects (checkouts and linked worktrees) under `root`
/// and fills `ProjectRow`/`WorktreeRow`. R3 then classifies artifact
/// directories at discovery, attributes each to its nearest containing
/// worktree, and folds the rest of every worktree into one `Source` row.
/// Growth and full Docker joins remain later slices (R4-R5); the one
/// Docker fact this pass understands is "no compose-project label
/// matches a discovered project name", which lands as an `UnownedRow`
/// with reason `OwnedByNothing` rather than being attributed anywhere.
pub fn report(root: &Path, docker_facts: Option<&Path>) -> Result<Report> {
    report_with(root, docker_facts, false, None, None)
}

/// Same as [`report`], optionally running `du -skPx` on the root as an
/// independent oracle for `reconciliation.du_total`, and optionally
/// observing into the reverse-delta growth store (`growth.rs`).
///
/// `store_dir` is the top-level `${SLOP_LIVIN_DIR}`-style directory; when
/// `None`, nothing is persisted and every `growth_bytes`/`regrowth_count`
/// stays at the R3 default (`None`/`0`) -- a read-only report, which is
/// what the golden test and `--no-observe` want. `since_override`
/// overrides the store's configured `since` setting for this call only
/// (`--since`).
pub fn report_with(
    root: &Path,
    docker_facts: Option<&Path>,
    verify_du: bool,
    store_dir: Option<&Path>,
    since_override: Option<&str>,
) -> Result<Report> {
    let discovered = crate::git::discover(root)?;
    let observed_at = crate::entities::now();

    // Group discovered checkouts/worktrees by project (object-store)
    // identity. A project's display name comes from its main checkout
    // when one was found under this root; otherwise it falls back to the
    // first worktree's own name (e.g. only a linked worktree was in scope).
    let mut projects: BTreeMap<String, ProjectRow> = BTreeMap::new();
    let mut worktree_paths: Vec<(PathBuf, String)> = Vec::new();
    for dw in discovered {
        let worktree_id = id_for(&dw.path.display().to_string());
        worktree_paths.push((dw.path.clone(), worktree_id.clone()));
        let entry = projects
            .entry(dw.project_id.clone())
            .or_insert_with(|| ProjectRow {
                project_id: dw.project_id.clone(),
                name: dw.project_name.clone(),
                worktrees: Vec::new(),
            });
        if dw.kind == WorktreeKind::Main {
            entry.name = dw.project_name.clone();
        }
        entry.worktrees.push(WorktreeRow {
            worktree_id,
            path: dw.path,
            kind: dw.kind,
            artifacts: Vec::new(),
            signals: Vec::new(),
        });
    }

    let worktree_refs: Vec<(&Path, &str)> = worktree_paths
        .iter()
        .map(|(p, id)| (p.as_path(), id.as_str()))
        .collect();
    let mut attribution = crate::attribution::attribute(root, &worktree_refs, observed_at);

    let mut projects: Vec<ProjectRow> = projects.into_values().collect();
    for project in &mut projects {
        for worktree in &mut project.worktrees {
            crate::attribution::apply_to_worktree(worktree, &mut attribution);
        }
    }

    let mut unowned = attribution.unowned;
    let mut docker_unowned_bytes = 0u64;
    if let Some(facts_path) = docker_facts {
        let project_names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();
        let (docker_unowned, bytes) = docker_unjoined_images(facts_path, &project_names);
        docker_unowned_bytes += bytes;
        unowned.extend(docker_unowned);
    }
    let _ = docker_unowned_bytes; // excluded from reconciliation: not part of the fs walk.

    let du = if verify_du {
        crate::attribution::du_total(root)
    } else {
        None
    };

    if let Some(dir) = store_dir {
        let volume_id = std::fs::metadata(root)
            .map(|m| std::os::unix::fs::MetadataExt::dev(&m))
            .unwrap_or(0);
        let config = crate::growth::load_config(dir);
        let since_secs = since_override
            .and_then(crate::growth::parse_duration_secs)
            .or_else(|| crate::growth::parse_duration_secs(&config.since))
            .unwrap_or(24 * 3600);
        crate::growth::observe_and_annotate(
            dir,
            volume_id,
            &mut projects,
            observed_at,
            config.retention_days,
            since_secs,
        )?;
    }

    Ok(Report {
        observed_at,
        root: root.to_path_buf(),
        projects,
        unowned,
        reconciliation: Reconciliation {
            attributed: attribution.attributed_total,
            unowned: attribution.unowned_total,
            walked_total: attribution.walked_total,
            du_total: du,
        },
    })
}

/// Minimal Docker-facts read for the one join R3's golden test pins:
/// an image whose `com.docker.compose.project` label does not match any
/// discovered project name is unowned, reason `OwnedByNothing`. Images
/// that *do* join a project, cache/volume rows, and growth accounting
/// are full Docker-join work for a later slice (R4/R5 per the epic) and
/// are intentionally left alone here.
fn docker_unjoined_images(facts_path: &Path, project_names: &[&str]) -> (Vec<UnownedRow>, u64) {
    let Ok(text) = std::fs::read_to_string(facts_path) else {
        return (Vec::new(), 0);
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return (Vec::new(), 0);
    };
    let mut rows = Vec::new();
    let mut bytes_total = 0u64;
    if let Some(images) = value.get("Images").and_then(|v| v.as_array()) {
        for image in images {
            let labels = image.get("Labels").and_then(|v| v.as_str()).unwrap_or("");
            let joined = project_names
                .iter()
                .any(|name| labels.contains(&format!("com.docker.compose.project={name}")));
            if joined {
                continue;
            }
            let id = image
                .get("ID")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown-image")
                .to_string();
            let bytes = image
                .get("Size")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            bytes_total += bytes;
            rows.push(UnownedRow {
                path_or_object: id,
                bytes,
                reason: UnownedReason::OwnedByNothing,
            });
        }
    }
    (rows, bytes_total)
}

pub fn to_json(report: &Report) -> Result<String> {
    Ok(serde_json::to_string_pretty(report)?)
}

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
            let kind = match worktree.kind {
                WorktreeKind::Main => "main",
                WorktreeKind::Linked => "linked",
            };
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
