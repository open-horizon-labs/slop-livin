//! The action layer for the agent operator (R7, #26): a **plan** is
//! proposed from report rows; a **grant** is written only by a human at
//! the CLI (`approve <plan>` for one plan, or a standing predicate grant);
//! **execute** re-derives every fact at the sink, moves to Trash, measures
//! the volume's free space before and after, and appends to the ledger.
//!
//! What is structurally impossible here, on purpose:
//! - no function in this module that an MCP tool can reach writes a grant;
//! - a plan can only contain folded artifact rows (dependency trees, build
//!   outputs, caches) — never a checkout, worktree, `.git`, Source tree,
//!   unowned path or Docker object; a "prune Docker" plan cannot be built;
//! - a plan is single-use and expires; a grant expires and has a byte budget;
//! - the tool never speaks in the verdict register: refusals name a fact.

use crate::filter::{Filter, Predicate};
use crate::ledger::{ActionRecord, Ledger};
use crate::report::{ArtifactKind, ArtifactRow, ProjectRow, Report, WorktreeRow};
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Plans expire 30 minutes after proposal: long enough for a human to
/// read and approve, short enough that the facts they rest on are recent.
pub const PLAN_TTL_SECS: u64 = 30 * 60;

fn now() -> u64 {
    crate::entities::now()
}

// ---------------------------------------------------------------------
// Plans
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlanStatus {
    Proposed,
    Executed,
}

/// One unit of action: exactly one folded artifact row, with the facts a
/// human needs to authorize it and the sink needs to re-derive it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanUnit {
    pub path: PathBuf,
    pub rel_path: String,
    pub project: String,
    pub project_id: String,
    pub worktree_id: String,
    pub worktree_path: PathBuf,
    pub kind: ArtifactKind,
    pub bytes: u64,
    pub growth_bytes: Option<i64>,
    pub regrowth_count: u32,
    pub observed_at: u64,
    /// Recovery contract of the kind, stated so the human sees *why* this
    /// verb applies: `local_rebuild` (build outputs, caches),
    /// `network_fetch` (dependency trees).
    pub recovery: String,
    pub idle_secs: Option<u64>,
    pub merge_complete: bool,
    /// Rendered signal values of the owning worktree at proposal time.
    pub signals: Vec<String>,
    /// `delete` (artifact or Source directory), `remove-worktree` (linked
    /// worktree), `archive` (whole checkout).
    pub verb: String,
    /// git tracking status of the path, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<crate::ignore::TrackState>,
    /// Facts the human must see before authorizing (dirty, unpushed,
    /// untracked content, no remote, git store). Stated, never enforced —
    /// the same line the TUI shows on its confirm prompt.
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub root: PathBuf,
    pub created_at: u64,
    pub expires_at: u64,
    pub proposed_by: String,
    pub status: PlanStatus,
    pub units: Vec<PlanUnit>,
    /// Rows the proposer asked for that were refused, with the fact that
    /// refused them. Kept on the plan so the human sees what was *not*
    /// proposed and why.
    pub refused: Vec<Refused>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Refused {
    pub path: PathBuf,
    pub cause: String,
}

impl Plan {
    pub fn planned_bytes(&self) -> u64 {
        self.units.iter().map(|u| u.bytes).sum()
    }
    pub fn is_expired(&self, at: u64) -> bool {
        at > self.expires_at
    }
}

fn plans_dir(dir: &Path) -> PathBuf {
    dir.join("plans")
}

fn plan_path(dir: &Path, id: &str) -> PathBuf {
    plans_dir(dir).join(format!("{id}.json"))
}

pub fn save_plan(dir: &Path, plan: &Plan) -> Result<()> {
    fs::create_dir_all(plans_dir(dir))?;
    let tmp = plan_path(dir, &plan.id).with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(plan)?)?;
    fs::rename(&tmp, plan_path(dir, &plan.id))?;
    Ok(())
}

pub fn load_plan(dir: &Path, id: &str) -> Result<Plan> {
    let p = plan_path(dir, id);
    let text =
        fs::read_to_string(&p).with_context(|| format!("no plan {id} under {}", p.display()))?;
    Ok(serde_json::from_str(&text)?)
}

pub fn list_plans(dir: &Path) -> Result<Vec<Plan>> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(plans_dir(dir)) else {
        return Ok(out);
    };
    for e in rd.flatten() {
        if e.path().extension().is_some_and(|x| x == "json")
            && let Ok(text) = fs::read_to_string(e.path())
            && let Ok(p) = serde_json::from_str::<Plan>(&text)
        {
            out.push(p);
        }
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(out)
}

/// The daemon-side removal for a Docker unit, or `None` for a unit that
/// is an ordinary path. The unit's "path" is the object's id or name.
fn docker_target(kind: &ArtifactKind, path: &Path) -> Option<crate::docker::Removal> {
    let id = path.display().to_string();
    match kind {
        ArtifactKind::DockerImage => Some(crate::docker::Removal::Image { id }),
        ArtifactKind::DockerVolume => Some(crate::docker::Removal::Volume { name: id }),
        _ => None,
    }
}

/// What may be planned: anything with a path on this filesystem. The
/// one refusal is Docker build cache, which the daemon exposes no
/// per-entry removal for. Everything else is the human's call, made with
/// the unit's `warnings` in front of them — including a Docker image or
/// volume, whose removal is permanent and says so.
pub fn refusal_for_kind(kind: &ArtifactKind) -> Option<&'static str> {
    match kind {
        // Docker exposes no per-record removal for build cache: only
        // `docker builder prune`, which acts on everything reclaimable at
        // once and so is a different unit of action than a plan unit.
        ArtifactKind::DockerBuildCache => Some(
            "docker has no per-entry build-cache removal; `docker builder prune` acts on all of it",
        ),
        // These two rows report bytes scattered across a checkout and
        // carry the worktree's own path. Acting on that path would take
        // the whole checkout, which is not what the row says.
        ArtifactKind::Ignored | ArtifactKind::Untracked => Some(
            "an aggregate of every such path under the checkout, not one directory; open the worktree and act on what is inside it",
        ),
        _ => None,
    }
}

fn recovery_for(kind: &ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::DependencyTree => "network_fetch",
        ArtifactKind::BuildOutput | ArtifactKind::Cache => "local_rebuild",
        ArtifactKind::Git => "irrecoverable",
        // Docker removals never reach Trash, so the contract is the only
        // thing standing between the human and a permanent loss.
        ArtifactKind::DockerImage => "pull_or_rebuild (permanent: no Trash)",
        ArtifactKind::DockerBuildCache => "local_rebuild (permanent: no Trash)",
        ArtifactKind::DockerVolume => "irrecoverable (permanent: no Trash, no copy anywhere)",
        // Ignored bytes are in no version control at all: nothing to
        // pull, nothing to check out again. Trash is the only copy.
        ArtifactKind::Ignored | ArtifactKind::Untracked => "irrecoverable outside Trash",
        ArtifactKind::Source | ArtifactKind::Loose | ArtifactKind::Unknown => "depends: see track",
    }
}

/// The facts a human weighs before authorizing a unit — the same line the
/// TUI puts on its confirm prompt.
fn warnings_for(wt: &WorktreeRow, a: &ArtifactRow, whole: Option<&ProjectRow>) -> Vec<String> {
    let mut w = Vec::new();
    match a.track {
        Some(crate::ignore::TrackState::Untracked) => {
            w.push("untracked: in no version control and under no ignore rule".into())
        }
        Some(crate::ignore::TrackState::Tracked) if a.kind == ArtifactKind::Source => {
            w.push("tracked source".into())
        }
        _ => {}
    }
    if a.kind == ArtifactKind::Git {
        w.push("git object store: history goes with it".into());
    }
    if let Some(p) = whole {
        // Whole-worktree/checkout unit: the worktree's own facts apply.
        for sig in &wt.signals {
            match (sig.name.as_str(), sig.value.as_str()) {
                ("dirty", "dirty") => w.push("dirty".into()),
                ("unpushed", v) if v != "0 unpushed" => w.push(v.to_string()),
                ("locked", "locked") => w.push("locked".into()),
                _ => {}
            }
        }
        if wt.kind != crate::report::WorktreeKind::Linked {
            if p.remote.is_none() {
                w.push("no remote to restore from".into());
            }
            for (path, bytes) in crate::ignore::untracked_content(&wt.path, 3, 100_000) {
                let rel = path
                    .strip_prefix(&wt.path)
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                w.push(format!(
                    "{rel} untracked {}",
                    crate::render::human_bytes_pub(bytes)
                ));
            }
        }
    }
    w
}

fn worktree_merge_complete(wt: &WorktreeRow) -> bool {
    wt.merge_complete
        .as_ref()
        .is_some_and(|m| m.verdict == crate::github::TriState::Yes)
}

fn signal_strings(wt: &WorktreeRow) -> Vec<String> {
    wt.signals.iter().map(|s| s.value.clone()).collect()
}

/// Builds a plan from report rows. `filter` narrows which artifact rows
/// are proposed (same grammar as `--filter`); `paths`, when non-empty,
/// restricts to those exact artifact paths. Rows of non-actionable kinds
/// are recorded under `refused`, never silently dropped. An empty plan is
/// an error, so a proposer cannot mint a plan that says nothing.
pub fn propose(
    report: &Report,
    filter: Option<&Filter>,
    paths: &[PathBuf],
    proposed_by: &str,
) -> Result<Plan> {
    let mut units = Vec::new();
    let mut refused = Vec::new();
    for project in &report.projects {
        for wt in &project.worktrees {
            for a in &wt.artifacts {
                // A Source row shares its path with the worktree root; when
                // that exact path is asked for, the human means the whole
                // worktree/checkout (handled below with its own verb).
                let is_worktree_root = a.kind.is_worktree_remainder() && a.path == wt.path;
                let wanted = if paths.is_empty() {
                    filter.is_none_or(|f| f.matches_artifact(project, a))
                } else {
                    !is_worktree_root && paths.iter().any(|p| p == &a.path)
                };
                if !wanted {
                    continue;
                }
                if let Some(cause) = refusal_for_kind(&a.kind) {
                    if !paths.is_empty() {
                        refused.push(Refused {
                            path: a.path.clone(),
                            cause: cause.to_string(),
                        });
                    }
                    continue;
                }
                units.push(unit_from_row(project, wt, a));
            }
        }
    }
    // Paths that name a worktree/checkout root or a Source directory.
    for p in paths {
        if units.iter().any(|u| &u.path == p) || refused.iter().any(|r| &r.path == p) {
            continue;
        }
        let mut found = false;
        'outer: for project in &report.projects {
            for wt in &project.worktrees {
                if &wt.path == p {
                    units.push(unit_from_worktree(project, wt));
                    found = true;
                    break 'outer;
                }
                if let Some(dirs) = report
                    .dirs_by_worktree
                    .as_ref()
                    .and_then(|m| m.get(&wt.worktree_id))
                    && let Some(d) = dirs.iter().find(|d| wt.path.join(&d.rel_path) == *p)
                {
                    units.push(unit_from_dir(project, wt, d));
                    found = true;
                    break 'outer;
                }
            }
        }
        if !found {
            refused.push(Refused {
                path: p.clone(),
                cause: "not a path in this report (artifact, Source directory, worktree or checkout); unowned paths and Docker objects are not plannable".into(),
            });
        }
    }
    if units.is_empty() {
        bail!(
            "nothing to propose: no plannable rows matched{}",
            if refused.is_empty() {
                String::new()
            } else {
                format!(
                    " ({} refused: {})",
                    refused.len(),
                    refused
                        .iter()
                        .map(|r| format!("{} — {}", r.path.display(), r.cause))
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            }
        );
    }
    units.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    let created_at = now();
    Ok(Plan {
        id: crate::entities::new_id(),
        root: report.root.clone(),
        created_at,
        expires_at: created_at + PLAN_TTL_SECS,
        proposed_by: proposed_by.to_string(),
        status: PlanStatus::Proposed,
        units,
        refused,
    })
}

fn unit_from_row(project: &ProjectRow, wt: &WorktreeRow, a: &ArtifactRow) -> PlanUnit {
    let rel = a
        .path
        .strip_prefix(&wt.path)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| a.path.display().to_string());
    PlanUnit {
        path: a.path.clone(),
        rel_path: rel,
        project: project.name.clone(),
        project_id: project.project_id.clone(),
        worktree_id: wt.worktree_id.clone(),
        worktree_path: wt.path.clone(),
        kind: a.kind.clone(),
        bytes: a.bytes,
        growth_bytes: a.growth_bytes,
        regrowth_count: a.regrowth_count,
        observed_at: a.observed_at,
        recovery: recovery_for(&a.kind).to_string(),
        idle_secs: wt.idle_secs,
        merge_complete: worktree_merge_complete(wt),
        signals: signal_strings(wt),
        verb: "delete".into(),
        track: a.track,
        warnings: warnings_for(wt, a, None),
    }
}

/// A whole worktree (linked → `remove-worktree`) or checkout (→ `archive`)
/// as one unit, when a proposer asks for the worktree path itself.
fn unit_from_worktree(project: &ProjectRow, wt: &WorktreeRow) -> PlanUnit {
    let bytes: u64 = wt.artifacts.iter().map(|a| a.bytes).sum();
    let linked = wt.kind == crate::report::WorktreeKind::Linked;
    let pseudo = ArtifactRow {
        kind: ArtifactKind::Source,
        path: wt.path.clone(),
        bytes,
        mtime_max: 0,
        ecosystem: None,
        hardlinked: false,
        local_bytes: bytes,
        track: None,
        growth_bytes: wt
            .artifacts
            .iter()
            .filter_map(|a| a.growth_bytes)
            .reduce(|x, y| x + y),
        regrowth_count: 0,
        observed_at: wt
            .artifacts
            .first()
            .map(|a| a.observed_at)
            .unwrap_or_else(now),
        confidence: crate::entities::Confidence::High,
        source: crate::report::Source::new("filesystem.walk"),
        note: None,
        created_at: None,
        containers: Vec::new(),
        shared_with: Vec::new(),
        dangling: false,
    };
    let mut u = unit_from_row(project, wt, &pseudo);
    u.rel_path = ".".into();
    u.verb = if linked { "remove-worktree" } else { "archive" }.into();
    u.recovery = if linked {
        "git worktree add again from the same repo".into()
    } else {
        project
            .remote
            .as_ref()
            .map(|r| format!("git clone {r}"))
            .unwrap_or_else(|| "none: no remote".into())
    };
    u.warnings = warnings_for(wt, &pseudo, Some(project));
    u
}

/// A Source directory (from the dir rollups) as one deletable unit.
fn unit_from_dir(project: &ProjectRow, wt: &WorktreeRow, d: &crate::report::DirRollup) -> PlanUnit {
    let pseudo = ArtifactRow {
        kind: ArtifactKind::Unknown,
        path: wt.path.join(&d.rel_path),
        bytes: d.allocated_total,
        mtime_max: 0,
        ecosystem: None,
        hardlinked: false,
        local_bytes: d.allocated_total,
        track: d.track,
        growth_bytes: None,
        regrowth_count: 0,
        observed_at: wt
            .artifacts
            .first()
            .map(|a| a.observed_at)
            .unwrap_or_else(now),
        confidence: crate::entities::Confidence::High,
        source: crate::report::Source::new("filesystem.walk"),
        note: None,
        created_at: None,
        containers: Vec::new(),
        shared_with: Vec::new(),
        dangling: false,
    };
    unit_from_row(project, wt, &pseudo)
}

// ---------------------------------------------------------------------
// Grants — written only by the human-facing CLI, never by MCP.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Grant {
    pub id: String,
    pub verb: String,
    /// Empty for a one-shot approval. Otherwise a `filter` expression
    /// restricted to unit predicates: `kind:`, `project:`, `idle > `,
    /// `merge-complete`.
    pub predicate: String,
    /// A one-shot approval covers exactly this plan and nothing else.
    pub plan_id: Option<String>,
    pub budget_bytes: Option<u64>,
    pub spent_bytes: u64,
    pub max_units: Option<u32>,
    pub used_units: u32,
    pub created_at: u64,
    pub expires_at: u64,
    pub actor: String,
    pub revoked: bool,
}

impl Grant {
    fn live(&self, at: u64) -> bool {
        !self.revoked && at <= self.expires_at
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct GrantFile {
    grants: Vec<Grant>,
}

fn grants_path(dir: &Path) -> PathBuf {
    dir.join("grants.json")
}

pub fn list_grants(dir: &Path) -> Result<Vec<Grant>> {
    let p = grants_path(dir);
    if !p.exists() {
        return Ok(vec![]);
    }
    let f: GrantFile = serde_json::from_str(&fs::read_to_string(p)?)?;
    Ok(f.grants)
}

fn write_grants(dir: &Path, grants: &[Grant]) -> Result<()> {
    fs::create_dir_all(dir)?;
    let tmp = grants_path(dir).with_extension("json.tmp");
    fs::write(
        &tmp,
        serde_json::to_vec_pretty(&GrantFile {
            grants: grants.to_vec(),
        })?,
    )?;
    fs::rename(&tmp, grants_path(dir))?;
    Ok(())
}

/// Validates a standing-grant predicate: only facts about the *unit*
/// (kind, project, idle, merge-complete). Growth windows and PR state are
/// report-time filters, not authorization terms.
pub fn validate_grant_predicate(expr: &str) -> Result<Filter> {
    let f = crate::filter::parse(expr)?;
    for p in &f.predicates {
        match p {
            Predicate::Kind(_)
            | Predicate::Project(_)
            | Predicate::Type(_)
            | Predicate::Size { .. }
            | Predicate::AgeGreaterThan(_)
            | Predicate::IdleGreaterThan(_)
            | Predicate::MergeComplete => {}
            Predicate::Growth { .. } => bail!(
                "grant predicates cannot use growth windows; use kind:/project:/idle >/merge-complete"
            ),
            Predicate::Pr(_) => {
                bail!("grant predicates cannot use pr:; use kind:/project:/idle >/merge-complete")
            }
        }
    }
    if f.predicates.is_empty() {
        bail!(
            "a standing grant needs at least one predicate (kind:, project:, idle > <dur>, merge-complete); a blank grant is a blank check"
        );
    }
    Ok(f)
}

/// Human-at-CLI: a standing grant. `expires_in_secs` and `budget_bytes`
/// are required so a grant can neither live forever nor be unbounded.
pub fn add_standing_grant(
    dir: &Path,
    predicate: &str,
    budget_bytes: u64,
    max_units: Option<u32>,
    expires_in_secs: u64,
    actor: &str,
) -> Result<Grant> {
    validate_grant_predicate(predicate)?;
    if budget_bytes == 0 {
        bail!("--budget is required and must be > 0");
    }
    if expires_in_secs == 0 {
        bail!("--expires is required and must be > 0");
    }
    let mut grants = list_grants(dir)?;
    let g = Grant {
        id: crate::entities::new_id(),
        verb: "delete".into(),
        predicate: predicate.trim().to_string(),
        plan_id: None,
        budget_bytes: Some(budget_bytes),
        spent_bytes: 0,
        max_units,
        used_units: 0,
        created_at: now(),
        expires_at: now() + expires_in_secs,
        actor: actor.to_string(),
        revoked: false,
    };
    grants.push(g.clone());
    write_grants(dir, &grants)?;
    Ok(g)
}

/// Human-at-CLI: approve one plan. The grant is scoped to that plan id and
/// expires with the plan.
pub fn approve(dir: &Path, plan_id: &str, actor: &str) -> Result<Grant> {
    let plan = load_plan(dir, plan_id)?;
    if plan.is_expired(now()) {
        bail!(
            "plan {plan_id} expired at {}; propose again",
            plan.expires_at
        );
    }
    if plan.status == PlanStatus::Executed {
        bail!("plan {plan_id} was already executed");
    }
    let mut grants = list_grants(dir)?;
    let g = Grant {
        id: crate::entities::new_id(),
        verb: "delete".into(),
        predicate: String::new(),
        plan_id: Some(plan_id.to_string()),
        budget_bytes: Some(plan.planned_bytes()),
        spent_bytes: 0,
        max_units: Some(plan.units.len() as u32),
        used_units: 0,
        created_at: now(),
        expires_at: plan.expires_at,
        actor: actor.to_string(),
        revoked: false,
    };
    grants.push(g.clone());
    write_grants(dir, &grants)?;
    Ok(g)
}

pub fn revoke_grant(dir: &Path, grant_id: &str) -> Result<()> {
    let mut grants = list_grants(dir)?;
    let g = grants
        .iter_mut()
        .find(|g| g.id == grant_id)
        .ok_or_else(|| anyhow!("no grant {grant_id}"))?;
    g.revoked = true;
    write_grants(dir, &grants)
}

/// Does this grant's predicate cover this unit? (Budget/unit caps are
/// checked separately at execution, cumulatively.)
fn grant_covers(g: &Grant, plan: &Plan, unit: &PlanUnit) -> bool {
    if let Some(pid) = &g.plan_id {
        return pid == &plan.id; // a one-shot approval covers the whole plan
    }
    if g.verb != unit.verb {
        return false;
    }
    let Ok(f) = crate::filter::parse(&g.predicate) else {
        return false;
    };
    f.predicates.iter().all(|p| match p {
        Predicate::Kind(k) => format!("{:?}", unit.kind).eq_ignore_ascii_case(k),
        Predicate::Project(name) => unit.project.eq_ignore_ascii_case(name),
        Predicate::Type(_) => true, // not carried on a unit; scope by project: instead
        Predicate::IdleGreaterThan(secs) => unit.idle_secs.is_some_and(|i| i > *secs),
        Predicate::MergeComplete => unit.merge_complete,
        Predicate::Size { greater, bytes } => {
            if *greater {
                unit.bytes > *bytes
            } else {
                unit.bytes < *bytes
            }
        }
        // Age is re-derived at the sink (`newest_mtime`), not trusted from
        // the plan: the grant covers the unit only if it is still that old.
        Predicate::AgeGreaterThan(secs) => newest_mtime(&unit.path, 2_000_000)
            .is_some_and(|m| crate::entities::now().saturating_sub(m) > *secs),
        Predicate::Growth { .. } | Predicate::Pr(_) => false,
    })
}

/// The exact command a human runs to authorize this plan. Printed in every
/// `awaiting-authorization` refusal so the agent can relay it verbatim.
pub fn approve_command(plan_id: &str) -> String {
    format!("slop-livin approve {plan_id}")
}

// ---------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitOutcome {
    pub path: PathBuf,
    pub kind: ArtifactKind,
    pub planned_bytes: u64,
    /// `completed` | `refused` | `failed`
    pub status: String,
    pub cause: Option<String>,
    pub grant_id: Option<String>,
    pub recovery_location: Option<PathBuf>,
    /// Compiled outputs copied to `<worktree>/bin/` before the unit went to
    /// Trash (`keep_executables`); empty when nothing applied.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preserved: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteResult {
    pub plan_id: String,
    /// `executed` | `awaiting-authorization` | `expired` | `already-executed`
    pub state: String,
    pub next_step: Option<String>,
    pub outcomes: Vec<UnitOutcome>,
    pub planned_bytes: u64,
    /// Bytes moved to Trash, which is to say recoverable.
    pub trashed_bytes: u64,
    /// Bytes removed with no Trash behind them: Docker objects, which the
    /// daemon deletes outright. Reported apart from `trashed_bytes`
    /// because the two mean opposite things to whoever has to undo this.
    #[serde(default)]
    pub removed_permanently_bytes: u64,
    pub free_before: Option<u64>,
    pub free_after: Option<u64>,
    /// Measured (free_after - free_before). Trash keeps the bytes on the
    /// volume, so this is expected to be ~0 until Trash is emptied; it is
    /// reported so nobody mistakes "trashed" for "freed".
    pub freed_measured: Option<i64>,
    pub actor: String,
}

/// Newest mtime anywhere under `path` (files and directories), bounded by
/// `max_entries` so a pathological tree cannot stall the sink; returns
/// `None` when the bound is hit (treated as "could not re-observe").
fn newest_mtime(path: &Path, max_entries: usize) -> Option<u64> {
    let mut newest = 0u64;
    let mut stack = vec![path.to_path_buf()];
    let mut seen = 0usize;
    while let Some(p) = stack.pop() {
        let meta = fs::symlink_metadata(&p).ok()?;
        let m = meta
            .modified()
            .ok()?
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_secs();
        newest = newest.max(m);
        if meta.is_dir() && !meta.file_type().is_symlink() {
            for e in fs::read_dir(&p).ok()?.flatten() {
                seen += 1;
                if seen > max_entries {
                    return None;
                }
                stack.push(e.path());
            }
        }
    }
    Some(newest)
}

pub fn trash_root() -> PathBuf {
    if let Ok(dir) = std::env::var("SLOP_LIVIN_TRASH_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".Trash")
}

pub fn free_space_bytes(path: &Path) -> Option<u64> {
    let out = std::process::Command::new("df")
        .arg("-k")
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let fields: Vec<&str> = text.lines().nth(1)?.split_whitespace().collect();
    let available_kb: u64 = fields.get(3)?.parse().ok()?;
    Some(available_kb * 1024)
}

fn ledger_path(dir: &Path) -> PathBuf {
    dir.join("ledger.jsonl")
}

/// Executes a plan. Every unit is checked independently; one refusal never
/// aborts the rest. Without a covering grant for any unit the plan is not
/// executed at all and the result names the command a human runs.
pub fn execute(dir: &Path, plan_id: &str, actor: &str) -> Result<ExecuteResult> {
    execute_with_trash(dir, plan_id, actor, &trash_root())
}

/// `execute`, first copying compiled outputs out of each unit into
/// `<worktree>/bin/` (see [`preserve_executables`]).
pub fn execute_keeping_executables(
    dir: &Path,
    plan_id: &str,
    actor: &str,
) -> Result<ExecuteResult> {
    execute_with_trash_opts(dir, plan_id, actor, &trash_root(), true)
}

/// A file preserved by `preserve_executables`: where it was, where it went.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preserved {
    pub from: PathBuf,
    pub to: PathBuf,
}

fn is_executable_file(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.is_file() && meta.permissions().mode() & 0o111 != 0
}

fn copy_into(from: &Path, dest_dir: &Path, out: &mut Vec<Preserved>) -> Result<()> {
    fs::create_dir_all(dest_dir)?;
    let name = from.file_name().context("file has a name")?;
    let to = dest_dir.join(name);
    fs::copy(from, &to).with_context(|| format!("copy {} to {}", from.display(), to.display()))?;
    out.push(Preserved {
        from: from.to_path_buf(),
        to,
    });
    Ok(())
}

/// Copies the compiled outputs a build directory holds to
/// `<worktree>/bin/` before the directory is trashed, the way
/// clean-dev-dirs' `--keep-executables` does:
///
/// - Rust `target/`: executables (mode +x, not `.d`/`.rlib`/`.rmeta`/
///   `.dylib`/`.so`/`.a`/`.pdb`) directly in `target/release/` and
///   `target/debug/` go to `bin/release/` and `bin/debug/`.
/// - Python `dist/`: `*.whl` (and `*.tar.gz`) go to `bin/`; `build/`:
///   `*.so`/`*.pyd` anywhere inside go to `bin/`.
/// - Everything else (dependency trees, caches, other build outputs) is a
///   no-op: nothing in them is an output worth keeping.
///
/// Returns what was copied. An empty list is a valid answer, never an error.
pub fn preserve_executables(unit_path: &Path, worktree: &Path) -> Result<Vec<Preserved>> {
    let mut out = Vec::new();
    let base = unit_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let bin = worktree.join("bin");
    const SKIP_EXT: &[&str] = &["d", "rlib", "rmeta", "a", "so", "dylib", "dll", "pdb"];
    match base {
        "target" => {
            for profile in ["release", "debug"] {
                let dir = unit_path.join(profile);
                let Ok(rd) = fs::read_dir(&dir) else { continue };
                for e in rd.flatten() {
                    let Ok(meta) = e.metadata() else { continue };
                    let p = e.path();
                    let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("");
                    if is_executable_file(&meta) && !SKIP_EXT.contains(&ext) {
                        copy_into(&p, &bin.join(profile), &mut out)?;
                    }
                }
            }
        }
        "dist" => {
            let Ok(rd) = fs::read_dir(unit_path) else {
                return Ok(out);
            };
            for e in rd.flatten() {
                let p = e.path();
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.ends_with(".whl") || name.ends_with(".tar.gz") {
                    copy_into(&p, &bin, &mut out)?;
                }
            }
        }
        "build" => {
            let mut stack = vec![unit_path.to_path_buf()];
            let mut seen = 0usize;
            while let Some(d) = stack.pop() {
                let Ok(rd) = fs::read_dir(&d) else { continue };
                for e in rd.flatten() {
                    seen += 1;
                    if seen > 200_000 {
                        return Ok(out);
                    }
                    let p = e.path();
                    let Ok(ft) = e.file_type() else { continue };
                    if ft.is_dir() {
                        stack.push(p);
                    } else if ft.is_file() {
                        let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("");
                        if ext == "so" || ext == "pyd" {
                            copy_into(&p, &bin, &mut out)?;
                        }
                    }
                }
            }
        }
        _ => {}
    }
    Ok(out)
}

/// `execute` with an explicit Trash root (tests; never process-global state).
pub fn execute_with_trash(
    dir: &Path,
    plan_id: &str,
    actor: &str,
    trash: &Path,
) -> Result<ExecuteResult> {
    execute_with_trash_opts(dir, plan_id, actor, trash, false)
}

pub fn execute_with_trash_opts(
    dir: &Path,
    plan_id: &str,
    actor: &str,
    trash: &Path,
    keep_executables: bool,
) -> Result<ExecuteResult> {
    let mut plan = load_plan(dir, plan_id)?;
    let at = now();
    let planned = plan.planned_bytes();
    let base = |state: &str, next: Option<String>| ExecuteResult {
        plan_id: plan_id.to_string(),
        state: state.into(),
        next_step: next,
        outcomes: vec![],
        planned_bytes: planned,
        trashed_bytes: 0,
        removed_permanently_bytes: 0,
        free_before: None,
        free_after: None,
        freed_measured: None,
        actor: actor.to_string(),
    };
    if plan.status == PlanStatus::Executed {
        return Ok(base("already-executed", Some("propose a new plan".into())));
    }
    if plan.is_expired(at) {
        return Ok(base(
            "expired",
            Some("propose again; plans live 30 minutes".into()),
        ));
    }
    let mut grants = list_grants(dir)?;
    // Authorization first, for every unit, before touching anything.
    let mut chosen: Vec<Option<usize>> = Vec::with_capacity(plan.units.len());
    for unit in &plan.units {
        let idx = grants
            .iter()
            .position(|g| g.live(at) && grant_covers(g, &plan, unit));
        chosen.push(idx);
    }
    if chosen.iter().all(|c| c.is_none()) {
        return Ok(base(
            "awaiting-authorization",
            Some(format!(
                "no grant covers this plan; a human runs `{}` (this plan only) or `slop-livin grant add '<kind:/project:/idle >/merge-complete>' --budget <size> --expires <dur>` (standing)",
                approve_command(plan_id)
            )),
        ));
    }

    let ledger = Ledger::open(ledger_path(dir))?;
    fs::create_dir_all(trash)?;
    let free_before = free_space_bytes(&plan.root);
    let mut outcomes = Vec::new();
    let mut trashed = 0u64;
    let mut removed_permanently = 0u64;
    let mut spent_by_grant: HashMap<usize, (u64, u32)> = HashMap::new();

    for (unit, choice) in plan.units.iter().zip(chosen.iter()) {
        let mut outcome = UnitOutcome {
            path: unit.path.clone(),
            kind: unit.kind.clone(),
            planned_bytes: unit.bytes,
            status: "refused".into(),
            cause: None,
            grant_id: None,
            recovery_location: None,
            preserved: Vec::new(),
        };
        let Some(gi) = *choice else {
            outcome.cause = Some(format!(
                "no grant covers this unit; `{}`",
                approve_command(plan_id)
            ));
            outcomes.push(outcome);
            continue;
        };
        let g = &grants[gi];
        outcome.grant_id = Some(g.id.clone());
        // Budget and unit caps, cumulative across this execution.
        let (spent, used) = spent_by_grant
            .get(&gi)
            .copied()
            .unwrap_or((g.spent_bytes, g.used_units));
        if let Some(b) = g.budget_bytes
            && spent + unit.bytes > b
        {
            outcome.cause = Some(format!(
                "grant budget exceeded: {} spent + {} unit > {} budget",
                spent, unit.bytes, b
            ));
            outcomes.push(outcome);
            continue;
        }
        if let Some(mu) = g.max_units
            && used + 1 > mu
        {
            outcome.cause = Some(format!("grant unit cap reached ({mu})"));
            outcomes.push(outcome);
            continue;
        }
        // A Docker object is not a path: it lives in the daemon, and it
        // is removed there, permanently. Same discipline — re-derived at
        // the sink, refused with the daemon's own words — but no Trash
        // and so no recovery location.
        if let Some(target) = docker_target(&unit.kind, &unit.path) {
            if let Err(why) = crate::docker::still_removable(&target) {
                outcome.cause = Some(why);
                outcomes.push(outcome);
                continue;
            }
            match crate::docker::remove(&target, std::time::Duration::from_secs(30)) {
                Ok(()) => {
                    outcome.status = "completed".into();
                    removed_permanently += unit.bytes;
                    spent_by_grant.insert(gi, (spent + unit.bytes, used + 1));
                    ledger.append(&crate::ledger::ActionRecord {
                        id: crate::entities::new_id(),
                        verb: crate::grants::Verb::Delete,
                        entity_id: crate::entities::id_for(&unit.path.display().to_string()),
                        evidence: serde_json::json!({
                            "plan_id": plan_id,
                            "kind": format!("{:?}", unit.kind),
                            "bytes": unit.bytes,
                            "recovery": unit.recovery,
                            "docker": format!("{target:?}"),
                            "permanent": true,
                        }),
                        grant_id: g.id.clone(),
                        actor: actor.to_string(),
                        outcome: "completed".to_string(),
                        recovery_location: None,
                        measured_free_space_delta: None,
                        observed_path_state: Some("removed via docker".to_string()),
                        recorded_at: at,
                    })?;
                }
                Err(why) => outcome.cause = Some(why),
            }
            outcomes.push(outcome);
            continue;
        }
        // Sink re-derivation: the path must still be the artifact it was.
        let Ok(meta) = fs::symlink_metadata(&unit.path) else {
            outcome.cause = Some("path no longer exists".into());
            outcomes.push(outcome);
            continue;
        };
        if !meta.is_dir() || meta.file_type().is_symlink() {
            outcome.cause = Some("path is no longer a directory (or is a symlink)".into());
            outcomes.push(outcome);
            continue;
        }
        let basename = unit.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if refusal_for_kind(&unit.kind).is_some() || basename.is_empty() {
            outcome.cause = Some("unit kind is not actionable".into());
            outcomes.push(outcome);
            continue;
        }
        // A linked worktree's `.git` is a file (gitdir pointer); a checkout's
        // is a directory. Both are whole-tree units and need `git worktree
        // prune` afterwards for the linked case.
        let linked_common: Option<PathBuf> = if unit.verb == "remove-worktree" {
            fs::read_to_string(unit.path.join(".git"))
                .ok()
                .and_then(|line| {
                    let gitdir = PathBuf::from(line.trim().strip_prefix("gitdir:")?.trim());
                    let gitdir = if gitdir.is_absolute() {
                        gitdir
                    } else {
                        unit.path.join(gitdir)
                    };
                    let c = fs::read_to_string(gitdir.join("commondir")).ok()?;
                    let c = PathBuf::from(c.trim());
                    Some(if c.is_absolute() { c } else { gitdir.join(c) })
                })
        } else {
            None
        };
        match newest_mtime(&unit.path, 2_000_000) {
            None => {
                outcome.cause = Some("could not re-observe the tree before acting".into());
                outcomes.push(outcome);
                continue;
            }
            Some(m) if m > plan.created_at => {
                outcome.cause = Some(format!(
                    "activity changed since plan: newest mtime {} > plan {} — propose again",
                    m, plan.created_at
                ));
                outcomes.push(outcome);
                continue;
            }
            Some(_) => {}
        }
        if keep_executables && unit.verb == "delete" {
            match preserve_executables(&unit.path, &unit.worktree_path) {
                Ok(kept) => outcome.preserved = kept.into_iter().map(|k| k.to).collect(),
                Err(e) => {
                    outcome.status = "failed".into();
                    outcome.cause = Some(format!("could not preserve executables: {e}"));
                    outcomes.push(outcome);
                    continue;
                }
            }
        }
        let dest = trash.join(format!(
            "{}-{}-{}",
            basename,
            unit.project.replace('/', "_"),
            at
        ));
        match fs::rename(&unit.path, &dest) {
            Ok(()) => {
                outcome.status = "completed".into();
                outcome.recovery_location = Some(dest.clone());
                trashed += unit.bytes;
                spent_by_grant.insert(gi, (spent + unit.bytes, used + 1));
                if let Some(c) = &linked_common {
                    let _ = std::process::Command::new("git")
                        .arg("-C")
                        .arg(c.parent().unwrap_or(c))
                        .args(["worktree", "prune"])
                        .output();
                }
            }
            Err(e) => {
                outcome.status = "failed".into();
                outcome.cause = Some(format!("rename to Trash failed: {e}"));
            }
        }
        ledger.append(&ActionRecord {
            id: crate::entities::new_id(),
            verb: match unit.verb.as_str() {
                "archive" => crate::grants::Verb::Archive,
                "remove-worktree" => crate::grants::Verb::RemoveWorktree,
                _ => crate::grants::Verb::Delete,
            },
            entity_id: crate::entities::id_for(&unit.path.display().to_string()),
            evidence: serde_json::json!({
                "plan_id": plan.id,
                "project": unit.project,
                "worktree": unit.worktree_path,
                "kind": format!("{:?}", unit.kind),
                "bytes": unit.bytes,
                "growth_bytes": unit.growth_bytes,
                "regrowth_count": unit.regrowth_count,
                "observed_at": unit.observed_at,
                "recovery": unit.recovery,
                "idle_secs": unit.idle_secs,
                "signals": unit.signals,
                "verb": unit.verb,
                "track": unit.track,
                "warnings_shown": unit.warnings,
            }),
            grant_id: g.id.clone(),
            actor: actor.to_string(),
            outcome: outcome.status.clone(),
            recovery_location: outcome.recovery_location.clone(),
            measured_free_space_delta: None,
            observed_path_state: Some(if outcome.status == "completed" {
                "trashed".into()
            } else {
                "unchanged".into()
            }),
            recorded_at: now(),
        })?;
        outcomes.push(outcome);
    }

    for (gi, (spent, used)) in spent_by_grant {
        grants[gi].spent_bytes = spent;
        grants[gi].used_units = used;
    }
    write_grants(dir, &grants)?;
    plan.status = PlanStatus::Executed;
    save_plan(dir, &plan)?;

    let free_after = free_space_bytes(&plan.root);
    Ok(ExecuteResult {
        plan_id: plan_id.to_string(),
        state: "executed".into(),
        next_step: match (
            outcomes.iter().any(|o| o.status == "completed"),
            trashed,
            removed_permanently,
        ) {
            (false, _, _) => Some("every unit was refused; read each cause".into()),
            (true, 0, _) => Some(
                "re-observe the affected worktrees; what the daemon removed is gone, not in Trash"
                    .into(),
            ),
            (true, _, 0) => {
                Some("re-observe the affected worktrees; bytes are in Trash until it is emptied".into())
            }
            (true, _, _) => Some(
                "re-observe the affected worktrees; the paths are in Trash until it is emptied, what the daemon removed is gone"
                    .into(),
            ),
        },
        outcomes,
        planned_bytes: planned,
        trashed_bytes: trashed,
        removed_permanently_bytes: removed_permanently,
        free_before,
        free_after,
        freed_measured: match (free_before, free_after) {
            (Some(b), Some(a)) => Some(a as i64 - b as i64),
            _ => None,
        },
        actor: actor.to_string(),
    })
}
