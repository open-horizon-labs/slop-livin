//! The action layer for the agent operator (R7, #26): a **plan** is
//! proposed from report rows; a **grant** is written only by a human at
//! the CLI (`approve <plan>` for one plan, or a standing predicate grant);
//! **execute** re-derives every fact at the sink, moves to Trash, measures
//! the volume's free space before and after, and appends to the ledger.
//!
//! What is structurally impossible here, on purpose:
//! - the functions that write a grant (`approve`, `add_standing_grant`,
//!   `revoke_grant`) are called only from the CLI's own approve/grant
//!   command handling or the TUI's confirmed-execution path -- see
//!   `.oh/guardrails/human-only-authorization.md` for the transport-
//!   independent statement of what that boundary actually is;
//! - a plan can only contain folded artifact rows (dependency trees, build
//!   outputs, caches) — never a checkout, worktree, `.git`, Source tree,
//!   unowned path or Docker object; a "prune Docker" plan cannot be built;
//! - a plan is single-use and expires; a grant expires and has a byte budget;
//! - the tool never speaks in the verdict register: refusals name a fact.

use crate::authority::{Authorized, HumanConfirmed};
use crate::filter::{Filter, Predicate};
use crate::fs_gate::{self, Metadata, MetadataExt, read::read_owned_string, store};
use crate::ledger::{ActionRecord, Ledger};
use crate::report::{ArtifactKind, ArtifactRow, ProjectRow, Report, WorktreeRow};
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cargo_group: Option<crate::cargo_cleanup::CargoGroup>,
    pub path: PathBuf,
    pub rel_path: String,
    pub project: String,
    pub project_id: String,
    pub worktree_id: String,
    pub worktree_path: PathBuf,
    pub kind: ArtifactKind,
    pub bytes: u64,
    #[serde(default)]
    pub dedup_stale: bool,
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
    /// Set only for a unit built from `external::ExternalUnit` (#43): the
    /// unit's storage category, stated so a plan can *name* an external
    /// unit for inspection/review without ever authorizing its removal.
    /// `execute` refuses every unit with this set, unconditionally,
    /// before grant/budget checks even run — registry/detector output
    /// is identification, never authorization, and this chunk ships no
    /// supported selective action for any external category.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_category: Option<String>,
    /// Set only for a unit built from `agents::AgentUnit` (#101): the
    /// facts `execute` needs to recheck occupancy/references and
    /// perform the one or two supported agent-storage actions (a
    /// single-path cache/log Trash move, or a multi-member session
    /// removal). Every other agent-storage unit (protected categories,
    /// unsupported categories, and anything reachable only via
    /// inspection) never reaches a `Plan` at all -- see
    /// `propose_agents`, which refuses those at proposal time rather
    /// than naming them here as inspection-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_meta: Option<AgentPlanMeta>,
    /// Decision evidence (#61): the exact activity/consumer/reclaimability
    /// facts this unit's report row already carried
    /// (`report::attach_decision_evidence`), plus a fresh current-use
    /// fact taken at proposal time. `execute` re-takes current-use fresh
    /// rather than trusting this snapshot -- see the occupancy recheck
    /// immediately before every rename/removal below.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<crate::evidence::Evidence>,
    /// What a human actually reviewed about this unit's storage: the
    /// anchor's `(device, inode)` plus its membership -- exactly for a
    /// bounded member set, a bounded summary plus a metadata fingerprint
    /// for a large cache. `execute` recomputes it and refuses on any
    /// drift, so an approval buys the *reviewed* bytes rather than
    /// whatever now occupies that path
    /// (`.oh/guardrails/execution-sinks-recheck-live-state.md`).
    ///
    /// `None` on a unit whose kind is never actionable (an external unit
    /// named for inspection). A `None` here on an actionable unit is
    /// itself a refusal at execution: a plan with no reviewed identity
    /// was never reviewed against live state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed: Option<crate::recheck::ReviewedIdentity>,
}

/// See `PlanUnit::agent_meta`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPlanMeta {
    pub tool_id: String,
    /// The tool's home directory, rechecked for occupancy immediately
    /// before acting (never assumed unchanged from proposal time).
    pub tool_home: PathBuf,
    pub category: String,
    /// `None`: a single-path Trash move of the unit's own `path` (a
    /// cache/log category directory). `Some`: a session removal --
    /// every listed member is moved together into one Trash envelope,
    /// after `execute` re-verifies each member still exists and belongs
    /// only to this session (see `crate::agents::claude_code`'s
    /// grouping rules, re-run fresh at execution rather than trusted
    /// from the plan).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_members: Option<Vec<PathBuf>>,
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
    /// Selection-set byte accounting (#59): this plan's naive per-unit
    /// sum next to the figure with storage already charged to an
    /// earlier-counted inode removed, plus whether any selected unit
    /// might share inodes whose membership was never enumerated. Carried
    /// on the plan (and so in the plan JSON a human or agent reads)
    /// because `planned_bytes` is deliberately the naive sum -- the
    /// number a grant budget is spent against -- and #59's whole point is
    /// that the same physical storage must not silently count twice in
    /// what a selection claims it would free.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<crate::reclaimability::SelectionEstimate>,
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
    store::write_json(
        store::JsonFile::Plan {
            store: dir,
            id: &plan.id,
        },
        plan,
    )?;
    Ok(())
}

pub fn load_plan(dir: &Path, id: &str) -> Result<Plan> {
    let p = plan_path(dir, id);
    let text =
        read_owned_string(&p).with_context(|| format!("no plan {id} under {}", p.display()))?;
    Ok(serde_json::from_str(&text)?)
}

pub fn list_plans(dir: &Path) -> Result<Vec<Plan>> {
    let mut out = Vec::new();
    for path in store::list_owned(plans_dir(dir)) {
        if path.extension().is_some_and(|x| x == "json")
            && let Ok(text) = read_owned_string(&path)
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
    if a.dedup_stale {
        w.push("unique-byte estimate is out of date; full scan required before budgeted standing-grant cleanup".into());
    }
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
                let rel = crate::scope::relative_to(&path, &wt.path)
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
        if let Some(nested) = report.nested_artifacts.iter().find(|u| &u.path == p) {
            let container = report
                .nested_artifacts
                .iter()
                .find(|u| Some(&u.id) == nested.container_id.as_ref());
            let owner = report
                .projects
                .iter()
                .flat_map(|project| project.worktrees.iter().map(move |wt| (project, wt)))
                .find(|(_, wt)| {
                    container.is_some_and(|c| wt.artifacts.iter().any(|a| a.path == c.path))
                });
            if let (Some(container), Some((project, wt))) = (container, owner) {
                match crate::cargo_cleanup::propose(&report.nested_artifacts, p, &container.path) {
                    Ok(group) => {
                        let row = wt
                            .artifacts
                            .iter()
                            .find(|a| a.path == container.path)
                            .unwrap();
                        let mut unit = unit_from_row(project, wt, row);
                        unit.path = p.clone();
                        // `unit_from_row` anchored the reviewed identity
                        // on the container (`target/`); the selection is
                        // this exact group path.
                        unit.reviewed = crate::recheck::capture_anchor(p).ok();
                        unit.rel_path = crate::scope::relative_to(p, &wt.path)
                            .unwrap_or(p)
                            .display()
                            .to_string();
                        unit.bytes = group.members.iter().map(|m| m.bytes).sum();
                        unit.dedup_stale = false; // selected members were freshly measured
                        unit.growth_bytes = nested.growth_bytes;
                        unit.verb = "cargo-group".into();
                        unit.recovery = "Trash envelope with restore.json; rebuilding may require unavailable source/toolchains".into();
                        unit.warnings = vec!["exact selected build, NOT proven obsolete; stop non-Cargo writers; advisory Cargo lock held during move".into()];
                        unit.warnings.push("size is selected allocation, not promised free space; moving to Trash does not free these bytes immediately".into());
                        if group.shared_storage {
                            unit.warnings.push("selected files have hardlinks; any links outside the selection remain intact and reclaimable space is unknown".into());
                        }
                        unit.warnings.extend(
                            group
                                .members
                                .iter()
                                .map(|m| format!("member: {}", m.path.display())),
                        );
                        unit.cargo_group = Some(group);
                        units.push(unit);
                    }
                    Err(e) => refused.push(Refused {
                        path: p.clone(),
                        cause: e.to_string(),
                    }),
                }
            } else {
                refused.push(Refused {
                    path: p.clone(),
                    cause: "nested artifact has no observed owning container".into(),
                });
            }
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
    if units.iter().any(|u| u.cargo_group.is_some()) {
        for (i, a) in units.iter().enumerate() {
            for b in units.iter().skip(i + 1) {
                if crate::scope::overlapping(&a.path, &b.path) {
                    bail!(
                        "overlapping cleanup selections; select either parent or child, not both"
                    );
                }
                if let (Some(a), Some(b)) = (&a.cargo_group, &b.cargo_group)
                    && a.members
                        .iter()
                        .any(|x| b.members.iter().any(|y| x.path == y.path))
                {
                    bail!("overlapping Cargo companion selections");
                }
            }
        }
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
        selection: Some(selection_estimate(&units)),
        units,
        refused,
    })
}

/// Same as [`propose`], additionally refusing any matched unit whose
/// path is human-protected (`swamp protect`, #60): a scanned project
/// file or an agent observation can never add or remove a protection --
/// only this explicit, human-controlled list (`agents::protect_add`/
/// `protect_remove`, itself callable only from the CLI's own `protect`
/// subcommand) does. A protected unit still appears in the plan's
/// `refused` list with a named cause, never silently dropped.
pub fn propose_checking_protection(
    report: &Report,
    filter: Option<&Filter>,
    paths: &[PathBuf],
    proposed_by: &str,
    protected: &[PathBuf],
) -> Result<Plan> {
    propose_refusing_protected(
        report,
        filter,
        paths,
        proposed_by,
        report.store_dir.as_deref(),
        protected,
    )
}

/// [`propose_checking_protection`] against the protect list in
/// `store_dir`, loaded fresh (an unreadable list refuses). What the CLI
/// calls: it has no raw copy of the list to pass, and needs none.
pub fn propose_checking_store_protection(
    report: &Report,
    filter: Option<&Filter>,
    paths: &[PathBuf],
    proposed_by: &str,
    store_dir: &Path,
) -> Result<Plan> {
    propose_refusing_protected(report, filter, paths, proposed_by, Some(store_dir), &[])
}

fn propose_refusing_protected(
    report: &Report,
    filter: Option<&Filter>,
    paths: &[PathBuf],
    proposed_by: &str,
    store_dir: Option<&Path>,
    protected: &[PathBuf],
) -> Result<Plan> {
    let mut plan = propose(report, filter, paths, proposed_by)?;
    // The caller's list is a convenience, never the authority. The PR
    // #123 review's counterexample: an unreadable `agent_protect.json`
    // reached this function as an *empty* list through the CLI's
    // `.unwrap_or_default()`, so a protected unit became plannable
    // exactly when protection state broke. When the report knows which
    // store it came from, protection is reloaded here and an error is a
    // refusal (`.oh/guardrails/protection-fails-closed.md`).
    let live = match store_dir {
        Some(dir) => crate::protection::load_protect(dir)?,
        None => crate::protection::ProtectList::empty(),
    };
    let protected = live.including(protected);
    if protected.is_empty() {
        return Ok(plan);
    }
    let mut kept = Vec::new();
    for u in plan.units {
        // Both directions, as everywhere else: a unit beneath a
        // protected path, and a unit that *contains* one.
        if protected.conflict(&u.path).is_some() {
            plan.refused.push(Refused {
                path: u.path.clone(),
                cause: "human-protected path (swamp protect); remove protection first if this unit should be actionable".into(),
            });
        } else {
            kept.push(u);
        }
    }
    plan.units = kept;
    if plan.units.is_empty() {
        bail!(
            "nothing to propose: every matched unit is human-protected ({} refused: {})",
            plan.refused.len(),
            plan.refused
                .iter()
                .map(|r| format!("{} — {}", r.path.display(), r.cause))
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
    Ok(plan)
}

/// Builds an inspection-only plan naming selected external units (#43).
/// Every resulting unit carries `external_category`, so `execute` refuses
/// all of them unconditionally: this exists so a human/agent can review
/// external storage through the same plan/ledger surface as everything
/// else, never to make it actionable. Mirrors `propose`'s "nothing
/// matched is an error, never a silent empty plan" contract.
pub fn propose_external(
    units: &[crate::external::ExternalUnit],
    paths: &[PathBuf],
    proposed_by: &str,
) -> Result<Plan> {
    let mut plan_units = Vec::new();
    let mut refused = Vec::new();
    for u in units {
        if !paths.is_empty() && !paths.iter().any(|p| p == &u.path) {
            continue;
        }
        plan_units.push(unit_from_external(u));
    }
    for p in paths {
        if !plan_units.iter().any(|u| &u.path == p) {
            refused.push(Refused {
                path: p.clone(),
                cause: "no external unit at this exact path in the current scope".to_string(),
            });
        }
    }
    if plan_units.is_empty() {
        bail!(
            "nothing to propose: no external unit matched{}",
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
    let created_at = now();
    Ok(Plan {
        id: crate::entities::new_id(),
        root: PathBuf::new(),
        created_at,
        expires_at: created_at + PLAN_TTL_SECS,
        proposed_by: proposed_by.to_string(),
        status: PlanStatus::Proposed,
        selection: Some(selection_estimate(&plan_units)),
        units: plan_units,
        refused,
    })
}

/// A proposal's evidence snapshot (#61): the report row's already-known
/// facts (Activity/Consumer/Recovery/Reclaimability, from
/// `report::attach_decision_evidence`) plus one fresh current-use
/// reading taken right now, at proposal time. `execute` never trusts
/// this snapshot's current-use entry -- it re-takes its own immediately
/// before acting (see the occupancy recheck in the rename/removal path
/// below), so a change between propose and execute is always caught
/// fresh rather than compared against a possibly-stale copy.
fn plan_unit_evidence(
    existing: &[crate::evidence::Evidence],
    path: &Path,
) -> Vec<crate::evidence::Evidence> {
    let mut evidence = existing.to_vec();
    evidence.push(crate::occupancy::open_file_evidence(path));
    evidence
}

/// Turns this plan's units into selection members for
/// `reclaimability::estimate_selection` (#59), supplying the physical
/// inode identities the plan *already reviewed* where it has them.
///
/// An ordinary filesystem artifact row records
/// `ReviewedMembership::Anchor`: one `stat` of the directory, no member
/// listing (enumerating a `target/` tree per matched row at proposal
/// time is exactly the cost `recheck::capture_anchor` exists to avoid).
/// A directory's own inode says nothing about which inodes hold its
/// bytes, so that unit contributes `inodes: None` -- summed as if
/// exclusive and flagged through `SelectionEstimate::unknown_sharing`
/// when the unit might hardlink, never quietly deduplicated against an
/// inode set nobody looked at. Agent and external units, whose member
/// sets are bounded by construction and recorded exactly, contribute
/// their real inodes and so are genuinely deduplicated.
fn selection_estimate(units: &[PlanUnit]) -> crate::reclaimability::SelectionEstimate {
    use crate::recheck::ReviewedMembership as M;
    let members: Vec<crate::reclaimability::SelectionMember> = units
        .iter()
        .map(|u| {
            let inodes = u.reviewed.as_ref().and_then(|r| match &r.membership {
                M::Exact { members } => Some(members.iter().map(|m| (r.device, m.inode)).collect()),
                // A single file's own inode *is* where its bytes live.
                M::File { .. } => Some(vec![(r.device, r.inode)]),
                // A directory anchor's inode is not; a bounded summary
                // deliberately did not record members.
                M::Anchor { .. } | M::Summary(_) => None,
            });
            crate::reclaimability::SelectionMember {
                label: u.rel_path.clone(),
                allocated_bytes: u.bytes,
                hardlinked: u.dedup_stale
                    || u.cargo_group.as_ref().is_some_and(|g| g.shared_storage),
                inodes,
            }
        })
        .collect();
    crate::reclaimability::estimate_selection(&members)
}

// ---------------------------------------------------------------------
// External-unit evidence at the proposal sink (#55, #58, #59)
//
// An external unit is identification-only until a human names it in a
// `propose`. That is deliberately where the live, per-unit checks below
// run: an occupancy/lock/device-state probe on every detected unit of
// every ordinary report would spawn processes during identification,
// which the occupancy discipline (and the TUI's non-blocking event path)
// forbid. Everything here is bounded -- fixed candidate filenames, one
// single-level `locations::shallow_list`, allow-listed read-only
// queries -- and none of it traverses.
// ---------------------------------------------------------------------

/// Lock-file names package and version managers conventionally create
/// *while they are working*, checked only as direct children of a
/// unit's own directory: Cargo's `$CARGO_HOME/.package-cache`, a generic
/// manager `.lock`, Gradle's cache journal and daemon registry locks,
/// and pnpm's store lock. This is a list of *conventions*, not a
/// detector table: a unit gets whichever of them actually exists in its
/// own directory, so a new detector for a tool that uses one of these
/// needs no change here.
const MANAGER_LOCK_FILENAMES: &[&str] = &[
    ".package-cache",
    ".lock",
    "journal-1.lock",
    "registry.bin.lock",
    "store.lock",
];

/// How many simulator devices one proposal probes. Each probe is one
/// bounded, allow-listed, read-only `xcrun simctl list devices -j`
/// (`locations::ALLOWED_COMMANDS`), run only because a human named this
/// unit; the cap keeps a store with an unusual number of devices from
/// turning one proposal into an unbounded number of subprocesses, and
/// the remainder is stated rather than dropped.
const SIMULATOR_DEVICE_PROBE_CAP: usize = 8;

/// Manager-lock current use (#55) for one external unit. Every existing
/// candidate lock is probed; when none of them exists the single
/// `Unknown` names what was looked for, so "no lock file" never reads as
/// "nothing is using this".
fn manager_lock_facts(unit: &crate::external::ExternalUnit) -> Vec<crate::evidence::Evidence> {
    let mut present = Vec::new();
    let mut absent = None;
    for name in MANAGER_LOCK_FILENAMES {
        let ev =
            crate::occupancy::manager_lock_evidence(&unit.detector_name, &unit.path.join(name));
        if matches!(ev.status, crate::evidence::FactStatus::Unknown { .. }) {
            absent = absent.or(Some(ev));
        } else {
            present.push(ev);
        }
    }
    if !present.is_empty() {
        return present;
    }
    absent
        .map(|ev| {
            vec![ev.with_note(format!(
                "no manager lock file present in this unit's own directory; looked for {}. A \
                 manager that only creates its lock while working leaves none here between \
                 operations, so this is not evidence that nothing is using the store",
                MANAGER_LOCK_FILENAMES.join(", ")
            ))]
        })
        .unwrap_or_default()
}

/// CoreSimulator names each device's data directory by its device UDID
/// (`8-4-4-4-12` hex). Recognizing the *shape* is what keeps an Android
/// AVD directory (`Pixel_5_API_31.avd`) out of a `simctl` query: both
/// are `Environments` units, and asking `simctl` about an AVD name would
/// return an answer about nothing.
fn is_simulator_device_udid(name: &str) -> bool {
    const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    let parts: Vec<&str> = name.split('-').collect();
    parts.len() == GROUPS.len()
        && parts
            .iter()
            .zip(GROUPS)
            .all(|(p, n)| p.len() == n && p.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Booted-device current use (#55) for a simulator device store: one
/// `simctl` reading per device directory the store holds, capped.
fn simulator_booted_facts(unit: &crate::external::ExternalUnit) -> Vec<crate::evidence::Evidence> {
    if unit.category != crate::locations::StorageCategory::Environments {
        return Vec::new();
    }
    let udids: Vec<String> = crate::locations::shallow_dir_names(&unit.path)
        .into_iter()
        .filter(|n| is_simulator_device_udid(n))
        .collect();
    if udids.is_empty() {
        return Vec::new();
    }
    let env = crate::locations::Environment::from_process();
    let mut out: Vec<crate::evidence::Evidence> = udids
        .iter()
        .take(SIMULATOR_DEVICE_PROBE_CAP)
        .map(|udid| {
            crate::occupancy::simulator_booted_evidence(udid, &env)
                .with_note(format!("device {udid}"))
        })
        .collect();
    if udids.len() > SIMULATOR_DEVICE_PROBE_CAP {
        out.push(crate::evidence::Evidence::unknown(
            crate::evidence::FactKind::CurrentUse,
            crate::evidence::FactSubtype::Booted,
            crate::evidence::EvidenceSource::ProcessQuery {
                tool: "xcrun simctl list devices -j".into(),
            },
            crate::entities::now(),
            crate::reason!(
                "{} device directories in this store; the first {SIMULATOR_DEVICE_PROBE_CAP} were \
                 read this pass and the rest were not asked about",
                udids.len()
            ),
        ));
    }
    out
}

/// Whether this unit's detector declares a Maven-layout dependency
/// store (`locations::StoreEntryLookup::MavenLayout`), read from the
/// registry rather than matched against a detector id -- so a second
/// Maven-layout store (a mirror, a `<localRepository>` override) is
/// recognized without editing anything here.
///
/// The fallback is the layout Maven itself documents and the detector
/// hard-codes (`~/.m2/repository`), used while the detector-declared
/// capability is still being filled in; see the session note.
fn is_maven_layout_store(unit: &crate::external::ExternalUnit) -> bool {
    use crate::locations::{ConventionRole, StoreEntryLookup};
    let registry = crate::locations::Registry::with_builtins();
    if let Some(d) = registry
        .detectors()
        .iter()
        .find(|d| d.id() == unit.detector_id)
        && d.manager_conventions().iter().any(|c| {
            matches!(
                c.role,
                ConventionRole::DependencyStore {
                    lookup: StoreEntryLookup::MavenLayout,
                    ..
                }
            )
        })
    {
        return true;
    }
    // No path-shape fallback. An earlier revision also matched
    // `.m2/repository` directly, because the Maven detector did not yet
    // declare its convention; it does now
    // (`locations::maven::MavenDetector::manager_conventions`), and a
    // path literal here would be exactly the wiring table
    // `.oh/guardrails/detector-ids-only-in-registry.md` exists to
    // prevent -- one that keeps working while the capability it
    // duplicates silently stops being declared.
    false
}

/// Recovery evidence (#58) for one external unit, selected by the unit's
/// own storage category plus the capabilities its detector declares --
/// never by a detector-id match.
fn external_recovery_facts(unit: &crate::external::ExternalUnit) -> Vec<crate::evidence::Evidence> {
    use crate::locations::{ConventionRole, StorageCategory};
    let carry = |r: crate::recovery::RecoveryAssessment| match &r.follow_up_check {
        Some(check) => r.evidence.clone().with_note(format!("check: {check}")),
        None => r.evidence,
    };
    let mut out = Vec::new();

    // Maven's local repository mixes downloaded and locally-`mvn
    // install`ed artifacts in one tree. At store granularity there is no
    // `_remote.repositories` marker to read, so the honest fact is the
    // stated limit, not a blanket `network_fetch` label for the whole
    // repository.
    if is_maven_layout_store(unit) {
        out.push(carry(crate::recovery::maven_artifact_recovery(false)));
    }

    // An installation store holds versions a manager can reinstall by
    // name. The manager is the one whose detector declares the
    // installed-versions convention; the versions are the store's own
    // single-level directory names, never a traversal.
    if unit.category == StorageCategory::Installation {
        let registry = crate::locations::Registry::with_builtins();
        let declares_versions = registry
            .detectors()
            .iter()
            .find(|d| d.id() == unit.detector_id)
            .is_some_and(|d| {
                d.manager_conventions()
                    .iter()
                    .any(|c| matches!(c.role, ConventionRole::DeclaredVersions { .. }))
            });
        if declares_versions {
            let installed = crate::locations::shallow_dir_names(&unit.path);
            if installed.is_empty() {
                out.push(carry(crate::recovery::toolchain_installation_recovery(
                    &unit.detector_name,
                    None,
                )));
            } else {
                for version in installed.iter().take(SIMULATOR_DEVICE_PROBE_CAP) {
                    out.push(carry(crate::recovery::toolchain_installation_recovery(
                        &unit.detector_name,
                        Some(version),
                    )));
                }
            }
        }
    }
    out
}

/// Byte accounting (#59) for one external unit whose storage is sparse:
/// a file's *apparent* length can be far larger than the blocks actually
/// charged on disk (a VM disk image is the canonical case). Removing it
/// only ever frees the allocated blocks -- the gap was never occupying
/// space -- so `sparse_file_accounting` keeps the two numbers separate
/// and never counts the apparent size as reclaimable.
///
/// One `symlink_metadata` for a file-backed unit, or one single-level
/// `locations::shallow_list` plus one `stat` per direct file child for a
/// directory-backed one. Never a traversal, and only for a unit a human
/// named.
fn sparse_byte_accounting_facts(
    unit: &crate::external::ExternalUnit,
) -> Vec<crate::evidence::Evidence> {
    let mut logical = 0u64;
    let mut allocated = 0u64;
    let mut add = |meta: &Metadata| {
        if meta.is_file() {
            logical = logical.saturating_add(meta.len());
            allocated = allocated.saturating_add(meta.blocks().saturating_mul(512));
        }
    };
    match fs_gate::symlink_metadata(&unit.path) {
        Ok(meta) if meta.is_file() => add(&meta),
        Ok(meta) if meta.is_dir() => {
            for entry in crate::locations::shallow_list(&unit.path) {
                if entry.is_dir {
                    continue;
                }
                if let Ok(m) = fs_gate::symlink_metadata(unit.path.join(&entry.name)) {
                    add(&m);
                }
            }
        }
        _ => return Vec::new(),
    }
    if logical <= allocated {
        // Nothing sparse to report: an ordinary dense file's apparent
        // length and its allocated blocks agree (up to block rounding,
        // which rounds allocation *up*).
        return Vec::new();
    }
    let acc = crate::reclaimability::sparse_file_accounting(logical, allocated);
    crate::reclaimability::accounting_evidence(
        &acc,
        crate::evidence::EvidenceSource::FilesystemMetadata {
            detail: "apparent length versus allocated blocks of this unit's own files (sparse \
                     storage; the unallocated gap was never occupying disk)"
                .into(),
        },
    )
}

fn unit_from_row(project: &ProjectRow, wt: &WorktreeRow, a: &ArtifactRow) -> PlanUnit {
    let rel = crate::scope::relative_to(&a.path, &wt.path)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| a.path.display().to_string());
    PlanUnit {
        cargo_group: None,
        path: a.path.clone(),
        rel_path: rel,
        project: project.name.clone(),
        project_id: project.project_id.clone(),
        worktree_id: wt.worktree_id.clone(),
        worktree_path: wt.path.clone(),
        kind: a.kind.clone(),
        bytes: a.bytes,
        dedup_stale: a.dedup_stale,
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
        evidence: plan_unit_evidence(&a.evidence, &a.path),
        external_category: None,
        agent_meta: None,
        reviewed: crate::recheck::capture_anchor(&a.path).ok(),
    }
}

/// A unit built from an `external::ExternalUnit` (#43): inspection-only,
/// by construction. There is no project/worktree to attribute it to (an
/// external unit's identity is independent of any project); the
/// placeholder fields below are stated honestly rather than borrowing a
/// real project/worktree identity that would misattribute it. `execute`
/// refuses every such unit unconditionally on `external_category`, so
/// none of the recovery/verb/grant machinery below is ever reachable for
/// it -- they are filled with inert, self-explanatory values only so the
/// plan is legible if a human inspects its JSON.
pub fn unit_from_external(unit: &crate::external::ExternalUnit) -> PlanUnit {
    let category = format!("{:?}", unit.category);
    // Live per-unit evidence, taken now because a human named this unit
    // (#55/#58/#59) -- see the module section above for why none of it
    // runs during identification.
    let mut evidence = unit.evidence.clone();
    evidence.extend(manager_lock_facts(unit));
    evidence.extend(simulator_booted_facts(unit));
    evidence.extend(external_recovery_facts(unit));
    evidence.extend(sparse_byte_accounting_facts(unit));
    PlanUnit {
        cargo_group: None,
        path: unit.path.clone(),
        rel_path: ".".into(),
        project: format!("(external: {})", unit.detector_name),
        project_id: format!("external:{}", unit.detector_id),
        worktree_id: format!("external:{}", unit.detector_id),
        worktree_path: unit.path.clone(),
        kind: ArtifactKind::Unknown,
        bytes: unit.bytes,
        dedup_stale: false,
        growth_bytes: unit.growth_bytes,
        regrowth_count: unit.regrowth_count,
        observed_at: unit.observed_at,
        recovery: format!("inspection only: no supported selective action for {category}"),
        idle_secs: None,
        merge_complete: false,
        signals: Vec::new(),
        verb: "inspect".into(),
        track: None,
        warnings: vec![format!(
            "external unit ({category}): identification only, never authorization"
        )],
        evidence,
        external_category: Some(category),
        agent_meta: None,
        // An external unit is refused unconditionally at execution, but
        // it still records what was reviewed: a later chunk that adds a
        // supported external action inherits the identity contract
        // rather than having to remember to add it.
        reviewed: crate::recheck::capture(&unit.path).ok(),
    }
}

// ---------------------------------------------------------------------
// Agent-storage actions (#101): cache/log Trash moves and explicit
// session removal, both through this same plan/grant/ledger/Trash path.
// Unlike `propose_external`, a supported unit here becomes a real,
// actionable `PlanUnit` -- but only after `agent_refusal` clears it, and
// `execute` re-derives occupancy/references/identity fresh rather than
// trusting anything set at proposal time.
// ---------------------------------------------------------------------

/// A file whose name suggests a SQLite database or one of its sidecar
/// files. Refused unconditionally (guardrail: "no individual WAL/SHM
/// deletion, no guessed SQLite cleanup") even though no named Claude
/// Code path is currently documented as SQLite -- defense in depth for
/// a future path this adapter has not been told about.
fn is_sqlite_like(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".sqlite")
        || lower.ends_with(".sqlite3")
        || lower.ends_with(".db")
        || lower.ends_with(".db-wal")
        || lower.ends_with(".db-shm")
        || lower.ends_with("-wal")
        || lower.ends_with("-shm")
}

/// Why `unit` cannot be proposed as an actionable agent-storage plan
/// unit, or `None` if it can. Checked again, independently, at
/// `execute` (occupancy and reference/identity re-derivation) -- this
/// function is the proposal-time gate, not a substitute for that recheck.
fn agent_refusal(u: &crate::agents::AgentUnit) -> Option<String> {
    if u.protected {
        let reason = u
            .protect_reason
            .clone()
            .unwrap_or_else(|| format!("{} is protected by default", u.category.label()));
        return Some(format!("protected: {reason}"));
    }
    if u.action == crate::agents::AgentActionCapability::None {
        return Some(format!(
            "no supported selective action for {} yet",
            u.category.label()
        ));
    }
    let touches_db_like = std::iter::once(&u.path)
        .chain(u.members.iter().map(|m| &m.path))
        .any(|p| is_sqlite_like(p));
    if touches_db_like {
        return Some(
            "touches a database-like (SQLite/WAL/SHM) file; never deleted individually".into(),
        );
    }
    // Proposal-time occupancy uses the same tri-state, descendant-aware
    // probe the sink uses, not a boolean on the anchor: a session whose
    // transcript is open *inside* a directory unit must not reach a plan
    // in the first place, and a probe that could not run must not read
    // as "nothing open" (`.oh/guardrails/occupancy-is-tristate-at-sinks.md`).
    let mut paths: Vec<PathBuf> = vec![u.path.clone()];
    paths.extend(u.members.iter().map(|m| m.path.clone()));
    match crate::recheck::member_occupancy(&paths) {
        crate::occupancy::OccupancyState::Free => {}
        other => return other.refusal(),
    }
    None
}

/// Builds a real, actionable plan from selected `AgentUnit`s (#101).
/// Every unit that reaches the plan carries `agent_meta`; anything
/// `agent_refusal` names is refused here, at proposal time, never
/// silently downgraded to an inspection-only row (that would be
/// `propose_external`'s contract, not this one's -- an agent-storage
/// plan either names a real, supported action or refuses).
pub fn propose_agents(
    units: &[crate::agents::AgentUnit],
    paths: &[PathBuf],
    proposed_by: &str,
) -> Result<Plan> {
    let mut plan_units = Vec::new();
    let mut refused = Vec::new();
    for u in units {
        if !paths.is_empty() && !paths.iter().any(|p| p == &u.path) {
            continue;
        }
        match agent_refusal(u) {
            Some(cause) => refused.push(Refused {
                path: u.path.clone(),
                cause,
            }),
            None => plan_units.push(unit_from_agent(u)),
        }
    }
    for p in paths {
        if !plan_units.iter().any(|u| &u.path == p) && !refused.iter().any(|r| &r.path == p) {
            refused.push(Refused {
                path: p.clone(),
                cause: "no agent-storage unit at this exact path in the current scope".to_string(),
            });
        }
    }
    if plan_units.is_empty() {
        bail!(
            "nothing to propose: no actionable agent-storage unit matched{}",
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
    // Defense in depth (#101's refusal matrix): today's adapters never
    // produce two agent-storage units whose own anchor paths nest
    // (each unit's identity is one category's own folded
    // directory/file), but nothing *enforces* that invariant across
    // fourteen independent adapters plus whatever a future one adds.
    // Refuse before a plan is minted, the same discipline `propose`'s
    // own Cargo-group overlap check already applies to filesystem
    // units, rather than silently accepting a plan whose execution
    // order could move a parent out from under a child (or vice
    // versa).
    for (i, a) in plan_units.iter().enumerate() {
        for b in plan_units.iter().skip(i + 1) {
            if crate::scope::overlapping(&a.path, &b.path) {
                bail!(
                    "overlapping agent-storage selections: {} and {} are nested (or identical); select either the parent or the child, not both",
                    a.path.display(),
                    b.path.display()
                );
            }
        }
    }
    let created_at = now();
    // `Plan.root` is one path; an agent-storage plan can in principle
    // span more than one tool home once a second adapter exists. Each
    // unit's own `agent_meta.tool_home` is the authoritative value
    // `execute` uses -- this field is informational (free-space
    // before/after) and set from the first matched unit's home.
    let root = plan_units
        .first()
        .and_then(|u: &PlanUnit| u.agent_meta.as_ref())
        .map(|m| m.tool_home.clone())
        .unwrap_or_default();
    Ok(Plan {
        id: crate::entities::new_id(),
        root,
        created_at,
        expires_at: created_at + PLAN_TTL_SECS,
        proposed_by: proposed_by.to_string(),
        status: PlanStatus::Proposed,
        selection: Some(selection_estimate(&plan_units)),
        units: plan_units,
        refused,
    })
}

fn unit_from_agent(u: &crate::agents::AgentUnit) -> PlanUnit {
    use crate::agents::{AgentActionCapability, ProjectLinkState};
    let category = u.category.label().to_string();
    let session_members = if u.action == AgentActionCapability::SessionRemoval {
        Some(u.members.iter().map(|m| m.path.clone()).collect())
    } else {
        None
    };
    let mut warnings = vec![format!(
        "agent-storage unit ({category}, tool {})",
        u.tool_name
    )];
    match u.action {
        AgentActionCapability::SessionRemoval => {
            warnings.push(
                "removes this session's resume/rewind/checkpoint history; the linked project's \
                 own files are untouched"
                    .into(),
            );
            if let ProjectLinkState::Linked { project_name, .. } = &u.project_link {
                warnings.push(format!("linked project: {project_name}"));
            }
            for m in &u.members {
                warnings.push(format!("member: {} ({:?})", m.path.display(), m.kind));
            }
        }
        AgentActionCapability::CacheOrLogTrash => {
            warnings.push(
                "recoverable Trash move; this category is regenerated automatically by the tool"
                    .into(),
            );
        }
        AgentActionCapability::None => {}
    }
    PlanUnit {
        cargo_group: None,
        path: u.path.clone(),
        rel_path: u.relative_path.clone(),
        project: match &u.project_link {
            ProjectLinkState::Linked { project_name, .. } => project_name.clone(),
            _ => format!("(agent: {})", u.tool_name),
        },
        project_id: format!("agent:{}", u.tool_id),
        worktree_id: format!("agent:{}:{}", u.tool_id, u.id),
        worktree_path: u.path.clone(),
        kind: ArtifactKind::Unknown,
        bytes: u.bytes,
        dedup_stale: false,
        growth_bytes: u.growth_bytes,
        regrowth_count: u.regrowth_count,
        observed_at: u.observed_at,
        recovery: match u.action {
            AgentActionCapability::CacheOrLogTrash => {
                "local_rebuild (regenerated by the tool)".to_string()
            }
            AgentActionCapability::SessionRemoval => {
                "irrecoverable outside Trash: unique conversation/checkpoint history".to_string()
            }
            AgentActionCapability::None => "inspection only".to_string(),
        },
        idle_secs: None,
        merge_complete: false,
        signals: Vec::new(),
        verb: "delete".into(),
        track: None,
        warnings,
        evidence: plan_unit_evidence(&u.evidence, &u.path),
        external_category: None,
        agent_meta: Some(AgentPlanMeta {
            tool_id: u.tool_id.clone(),
            tool_home: u.tool_home.clone(),
            category,
            session_members,
        }),
        reviewed: crate::recheck::capture(&u.path).ok(),
    }
}

/// A single-path Trash move of a cache/log category directory.
///
/// Before the move it runs the full live-state recheck
/// (`.oh/guardrails/execution-sinks-recheck-live-state.md`): the
/// directory must still be the one that was reviewed, with the exact
/// membership that was reviewed (`reviewed_snapshot`); no human
/// keep/protect entry may cover it or anything under it, loaded fresh
/// (`live_protection`); and nothing may hold any member open, where an
/// unanswerable probe refuses (`member_occupancy`). `is_dir()` alone was
/// what let a replaced directory spend an old approval.
fn execute_agent_cache_trash(
    store_dir: &Path,
    path: &Path,
    reviewed: Option<&crate::recheck::ReviewedIdentity>,
    trash: &Path,
    at: u64,
    auth: &Authorized,
) -> Result<(PathBuf, u64)> {
    let proof = crate::recheck::run_all(store_dir, path, reviewed, &[])?;
    if !proof.identity().is_dir {
        bail!("path is no longer a directory (or is a symlink)");
    }
    let (bytes, _mtime, _truncated) = crate::agents::folded_bytes(path, 2_000_000);
    let basename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("agent-cache");
    let dest =
        fs_gate::destroy::trash_move(proof, auth, trash, &format!("agent-cache-{basename}-{at}"))?;
    Ok((dest, bytes))
}

/// One member's fate inside a session-removal Trash envelope's own
/// recovery manifest (`restore.json`, mirroring `cargo_cleanup`'s own
/// precedent of writing a manifest into the envelope it creates).
/// Written *before* any member is moved (every entry `"pending"`) and
/// rewritten after each successful move, so a partial failure (some
/// members moved, then a rename fails) still leaves an accurate,
/// on-disk account of exactly what happened -- never just an error
/// message with no durable record next to the moved content itself.
#[derive(Debug, Clone, Serialize)]
struct RestoreManifestMember {
    /// Absolute original path, so a human/script can restore it.
    original: PathBuf,
    /// Name inside the envelope once moved; `None` while `"pending"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    moved_to: Option<String>,
    bytes: u64,
    /// `"pending"` | `"moved"`.
    status: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct RestoreManifest {
    tool_id: String,
    category: String,
    session_path: PathBuf,
    members: Vec<RestoreManifestMember>,
}

fn write_restore_manifest(
    envelope: &fs_gate::destroy::Envelope,
    manifest: &RestoreManifest,
) -> Result<()> {
    envelope.write_manifest(manifest)
}

/// Returned when a session removal fails *after* the Trash envelope was
/// created and at least the pre-flight pass completed -- i.e. some
/// members may already be physically inside `envelope`. Carries enough
/// for the caller to still record an honest `recovery_location` and
/// partial `trashed_bytes` rather than silently losing track of content
/// that really did move. See `restore.json` inside `envelope` for the
/// exact per-member outcome.
#[derive(Debug)]
pub struct PartialAgentRemoval {
    pub envelope: PathBuf,
    pub moved_bytes: u64,
    pub source: anyhow::Error,
}

impl std::fmt::Display for PartialAgentRemoval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({} already moved into {}; see restore.json there for the exact per-member outcome)",
            self.source,
            crate::render::human_bytes_pub(self.moved_bytes),
            self.envelope.display()
        )
    }
}

impl std::error::Error for PartialAgentRemoval {}

/// Moves a session's exact member set into one Trash envelope, after
/// re-deriving the session's current membership from scratch (never
/// trusting `planned_members`) and refusing on any drift: a member now
/// missing, a new member the plan did not know about, or membership
/// that no longer matches at all (the session was already removed,
/// re-created, or reclassified since the plan was proposed).
#[allow(clippy::too_many_arguments)]
fn execute_agent_session_removal(
    store_dir: &Path,
    meta: &AgentPlanMeta,
    session_path: &Path,
    planned_members: &[PathBuf],
    reviewed: Option<&crate::recheck::ReviewedIdentity>,
    trash: &Path,
    at: u64,
    auth: &Authorized,
) -> Result<(PathBuf, u64)> {
    // One registry dispatch, never a second fourteen-arm tool-id match:
    // this used to be its own copy of `agents::identify_for_tool`'s
    // table, and a tool added to one and not the other identified fine
    // and then refused to re-verify here
    // (`.oh/guardrails/agent-adapters-are-pluggable.md`).
    //
    // The registry's recheck path runs with the identification cache
    // disabled, so this is live state rather than a cached derivation.
    let fresh = crate::agents::reidentify_for_tool(&meta.tool_id, &meta.tool_home, at).ok_or_else(
        || {
            anyhow!(
                "no session-removal re-identification implemented for tool {}",
                meta.tool_id
            )
        },
    )?;
    let current = fresh
        .iter()
        .find(|c| c.path() == session_path)
        .ok_or_else(|| anyhow!("session no longer identifiable at this path; propose again"))?;
    let mut current_members: Vec<PathBuf> =
        current.members().iter().map(|m| m.path.clone()).collect();
    let mut planned: Vec<PathBuf> = planned_members.to_vec();
    current_members.sort();
    planned.sort();
    if current_members != planned {
        bail!(
            "session membership changed since the plan was proposed (references drifted); \
             propose again"
        );
    }
    // The shared live-state recheck, over the session's anchor *and*
    // every member (`.oh/guardrails/execution-sinks-recheck-live-state.md`).
    // The membership comparison above catches references drifting; this
    // catches the anchor being replaced, a protect entry added after
    // approval in either direction, and anything holding a member open
    // -- including an unanswerable occupancy probe, which refuses.
    let proof = crate::recheck::run_all(store_dir, session_path, reviewed, &current_members)?;

    // Pre-flight: stat every member *before* moving any of them, so the
    // common failure (a member vanished between proposal and execution)
    // is caught before this session is left half-moved. This does not
    // make the multi-file move fully atomic (a concurrent deletion or a
    // cross-device rename can still fail mid-loop), but it removes the
    // most likely partial-failure cause outright.
    let mut sized_members: Vec<(PathBuf, u64, bool)> = Vec::with_capacity(current_members.len());
    for member in &current_members {
        let member_meta = fs_gate::symlink_metadata(member)
            .with_context(|| format!("member no longer exists: {}", member.display()))?;
        let is_dir = member_meta.is_dir();
        let bytes = if is_dir {
            crate::agents::folded_bytes(member, 2_000_000).0
        } else {
            member_meta.len()
        };
        sized_members.push((member.clone(), bytes, is_dir));
    }

    let slug = session_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("session");
    let mut envelope = fs_gate::destroy::Envelope::open(
        proof,
        auth,
        trash,
        &format!("agent-session-{slug}-{at}"),
        None,
    )?;

    let mut manifest = RestoreManifest {
        tool_id: meta.tool_id.clone(),
        category: meta.category.clone(),
        session_path: session_path.to_path_buf(),
        members: sized_members
            .iter()
            .map(|(path, bytes, _)| RestoreManifestMember {
                original: path.clone(),
                moved_to: None,
                bytes: *bytes,
                status: "pending",
            })
            .collect(),
    };
    // Written before any move, so even a failure on the very first
    // member leaves an accurate (all-pending) manifest next to whatever
    // Trash envelope directory was created.
    write_restore_manifest(&envelope, &manifest)?;

    let mut moved_bytes = 0u64;
    for (i, (member, bytes, _is_dir)) in sized_members.iter().enumerate() {
        let name = member
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("member");
        let dest_name = format!("{i}-{name}");
        if let Err(e) = envelope.move_member(member, &dest_name).with_context(|| {
            format!(
                "rename to Trash failed for {} ({} of {} members already moved into {})",
                member.display(),
                i,
                sized_members.len(),
                envelope.path().display()
            )
        }) {
            // Best-effort: leave the manifest reflecting exactly what
            // moved before this failure, never silently stale.
            let _ = write_restore_manifest(&envelope, &manifest);
            return Err(PartialAgentRemoval {
                envelope: envelope.path().to_path_buf(),
                moved_bytes,
                source: e,
            }
            .into());
        }
        moved_bytes += bytes;
        manifest.members[i].status = "moved";
        manifest.members[i].moved_to = Some(dest_name);
        // Rewritten after every successful move (not only at the end),
        // so a failure on member i+1 still leaves an accurate record of
        // members 0..=i having actually moved.
        write_restore_manifest(&envelope, &manifest)?;
    }
    Ok((envelope.path().to_path_buf(), moved_bytes))
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
        dedup_stale: wt.artifacts.iter().any(|a| a.dedup_stale),
        allocated_bytes: None,
        allocated_growth_bytes: None,
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
        evidence: Vec::new(),
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
        dedup_stale: false,
        allocated_bytes: None,
        allocated_growth_bytes: None,
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
        evidence: Vec::new(),
    };
    unit_from_row(project, wt, &pseudo)
}

// ---------------------------------------------------------------------
// Grants — minted only from the reviewed CLI approve/grant command
// handling or the TUI's confirmed-execution path (human_only_authorization).
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

/// Whether `g` is live at `at` and its predicate covers `unit` of `plan`:
/// the grant half of [`crate::authority::authorize`].
pub(crate) fn grant_is_live_and_covers(g: &Grant, plan: &Plan, unit: &PlanUnit, at: u64) -> bool {
    g.live(at) && grant_covers(g, plan, unit)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct GrantFile {
    grants: Vec<Grant>,
}

fn grants_path(dir: &Path) -> PathBuf {
    dir.join("grants.json")
}

pub fn list_grants(dir: &Path) -> Result<Vec<Grant>> {
    let text = match read_owned_string(grants_path(dir)) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.into()),
    };
    let f: GrantFile = serde_json::from_str(&text)?;
    Ok(f.grants)
}

fn write_grants(dir: &Path, grants: &[Grant]) -> Result<()> {
    store::write_json(
        store::JsonFile::Grants { store: dir },
        &GrantFile {
            grants: grants.to_vec(),
        },
    )?;
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
///
/// Takes the [`HumanConfirmed`] the CLI's `grant add` handler minted: a
/// caller that does not hold one (a keystroke handler, an agent path, a
/// convenience wrapper) does not compile
/// (`.oh/guardrails/human-only-authorization.md`).
pub fn add_standing_grant_confirmed(
    dir: &Path,
    predicate: &str,
    budget_bytes: u64,
    max_units: Option<u32>,
    expires_in_secs: u64,
    confirmed: &HumanConfirmed,
) -> Result<Grant> {
    let actor = confirmed.actor();
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

/// The test-fixture spelling of [`add_standing_grant_confirmed`], for
/// integration tests only: exists only with the `testing` feature, which
/// no production build enables (`crates/core/Cargo.toml`).
#[cfg(feature = "testing")]
pub fn add_standing_grant(
    dir: &Path,
    predicate: &str,
    budget_bytes: u64,
    max_units: Option<u32>,
    expires_in_secs: u64,
    actor: &str,
) -> Result<Grant> {
    add_standing_grant_confirmed(
        dir,
        predicate,
        budget_bytes,
        max_units,
        expires_in_secs,
        &HumanConfirmed::cli_command(actor),
    )
}

/// The test-fixture spelling of [`approve_confirmed`]; `testing` only,
/// like [`add_standing_grant`]. The reviewers' counterexample files call
/// it by this name.
#[cfg(feature = "testing")]
pub fn approve(dir: &Path, plan_id: &str, actor: &str) -> Result<Grant> {
    approve_confirmed(dir, plan_id, &HumanConfirmed::cli_command(actor))
}

/// Human-at-CLI (or the TUI's confirm dialog): approve one plan. The
/// grant is scoped to that plan id and expires with the plan. Takes the
/// [`HumanConfirmed`] the confirmation site minted.
pub fn approve_confirmed(dir: &Path, plan_id: &str, confirmed: &HumanConfirmed) -> Result<Grant> {
    let actor = confirmed.actor();
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
    if unit.cargo_group.is_some() || unit.dedup_stale {
        return false;
    } // explicit per-plan approval only
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
        Predicate::AgeGreaterThan(secs) => crate::recheck::newest_mtime(&unit.path, 2_000_000)
            .is_some_and(|m| crate::entities::now().saturating_sub(m) > *secs),
        Predicate::Growth { .. } | Predicate::Pr(_) => false,
    })
}

/// The exact command a human runs to authorize this plan. Printed in every
/// `awaiting-authorization` refusal so the agent can relay it verbatim.
pub fn approve_command(plan_id: &str) -> String {
    format!("swamp approve {plan_id}")
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
    /// Outcome evidence (#59): the one *measured* reclaimability number
    /// in this pipeline -- a real `statvfs`/`df` reading taken before
    /// and after this execution, carrying its own limits (a move to
    /// Trash on the same volume, an open file another process still
    /// holds, a filesystem snapshot or a concurrent writer can each
    /// suppress the change). Deliberately a separate fact from the
    /// plan's `EstimatedReclaimable`: an estimate and an observation are
    /// different things, and collapsing them is how a scan comes to
    /// promise exact reclaimed bytes. Empty when nothing was executed --
    /// an expired, already-executed or unauthorized plan measures
    /// nothing rather than reporting a fabricated zero.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<crate::evidence::Evidence>,
    pub actor: String,
}

pub fn trash_root() -> PathBuf {
    if let Ok(dir) = std::env::var("SWAMP_TRASH_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".Trash")
}

pub fn free_space_bytes(path: &Path) -> Option<u64> {
    let out = crate::fs_gate::spawn::run(
        crate::fs_gate::spawn::Program::Df,
        [std::ffi::OsStr::new("-k"), path.as_os_str()],
        std::time::Duration::from_secs(10),
    )
    .ok()?;
    if !out.success() {
        return None;
    }
    let text = out.stdout_lossy();
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

pub use crate::preserve::{Preserved, preserve_executables};

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
        evidence: Vec::new(),
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
                "no grant covers this plan; a human runs `{}` (this plan only) or `swamp grant add '<kind:/project:/idle >/merge-complete>' --budget <size> --expires <dur>` (standing)",
                approve_command(plan_id)
            )),
        ));
    }

    let ledger = Ledger::open(ledger_path(dir))?;
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
        if let Some(category) = &unit.external_category {
            outcome.cause = Some(format!(
                "no supported selective action for {category}: external units are inspection-only"
            ));
            outcomes.push(outcome);
            continue;
        }
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
        // The value every destructive step below borrows: this grant,
        // live, covering this unit, within budget. Minted only here.
        let Some(auth) = crate::authority::authorize(&plan, unit, g, spent, used, at) else {
            outcome.cause = Some(format!(
                "no live grant covers this unit; `{}`",
                approve_command(plan_id)
            ));
            outcomes.push(outcome);
            continue;
        };
        // Agent-storage action (#101): occupancy, reference and identity
        // are all rechecked fresh here, never trusted from the plan.
        if let Some(meta) = &unit.agent_meta {
            let agent_result = match &meta.session_members {
                Some(planned_members) => execute_agent_session_removal(
                    dir,
                    meta,
                    &unit.path,
                    planned_members,
                    unit.reviewed.as_ref(),
                    trash,
                    at,
                    &auth,
                ),
                None => execute_agent_cache_trash(
                    dir,
                    &unit.path,
                    unit.reviewed.as_ref(),
                    trash,
                    at,
                    &auth,
                ),
            };
            match agent_result {
                Ok((dest, moved_bytes)) => {
                    outcome.status = "completed".into();
                    outcome.recovery_location = Some(dest);
                    trashed += moved_bytes;
                    spent_by_grant.insert(gi, (spent + unit.bytes, used + 1));
                }
                Err(e) => {
                    outcome.status = "failed".into();
                    // A partial session removal (some members already
                    // physically moved before a later rename failed)
                    // still names its Trash envelope and the bytes that
                    // really did move -- `restore.json` inside that
                    // envelope has the exact per-member account. Never
                    // silently drop where partially-moved content went.
                    if let Some(partial) = e.downcast_ref::<PartialAgentRemoval>() {
                        outcome.recovery_location = Some(partial.envelope.clone());
                        trashed += partial.moved_bytes;
                    }
                    outcome.cause = Some(e.to_string());
                }
            }
            ledger.append(&ActionRecord {
                id: crate::entities::new_id(),
                verb: crate::grants::Verb::Delete,
                entity_id: crate::entities::id_for(&unit.path.display().to_string()),
                evidence: serde_json::json!({
                    "plan_id": plan.id,
                    "tool_id": meta.tool_id,
                    "category": meta.category,
                    "session_removal": meta.session_members.is_some(),
                    "member_count": meta.session_members.as_ref().map(|m| m.len()),
                    "bytes": unit.bytes,
                    "recovery": unit.recovery,
                    "cause": outcome.cause,
                }),
                grant_id: g.id.clone(),
                actor: actor.into(),
                outcome: outcome.status.clone(),
                recovery_location: outcome.recovery_location.clone(),
                measured_free_space_delta: None,
                observed_path_state: Some(if outcome.status == "completed" {
                    "trashed".into()
                } else {
                    "unchanged".into()
                }),
                recorded_at: at,
            })?;
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
            match crate::docker::remove(&target, &auth, &unit.path) {
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
        if let Some(group) = &unit.cargo_group {
            if keep_executables {
                outcome.cause =
                    Some("keep-executables conflicts with selective executable removal".into());
                outcomes.push(outcome);
                continue;
            }
            // The Cargo group's own recheck lives in
            // `cargo_cleanup::move_reviewed`, next to the rename it
            // guards: role, fingerprint, membership, identity and member
            // *contents* under a held Cargo build lock -- a stronger
            // identity check than the shared one, affordable because
            // these are small build outputs with no privacy constraint.
            // The two thirds it was missing (protection loaded fresh in
            // both directions, tri-state occupancy) moved in there too,
            // so one function owns the whole gate.
            ledger.append(&ActionRecord {
                id: crate::entities::new_id(),
                verb: crate::grants::Verb::Delete,
                entity_id: crate::entities::id_for(&unit.path.display().to_string()),
                evidence: serde_json::json!({"plan_id":plan.id,"cargo_group":group}),
                grant_id: g.id.clone(),
                actor: actor.into(),
                outcome: "intent".into(),
                recovery_location: None,
                measured_free_space_delta: None,
                observed_path_state: Some("preflight".into()),
                recorded_at: at,
            })?;
            match crate::cargo_cleanup::move_reviewed(
                dir,
                group,
                unit.reviewed.as_ref(),
                trash,
                &auth,
            ) {
                Ok(dest) => {
                    outcome.status = "completed".into();
                    outcome.recovery_location = Some(dest);
                    trashed += unit.bytes;
                    spent_by_grant.insert(gi, (spent + unit.bytes, used + 1));
                }
                Err(e) => {
                    outcome.status = "failed".into();
                    outcome.cause = Some(e.to_string());
                }
            }
            ledger.append(&ActionRecord {
                id:crate::entities::new_id(),verb:crate::grants::Verb::Delete,entity_id:crate::entities::id_for(&unit.path.display().to_string()),
                evidence:serde_json::json!({"plan_id":plan.id,"cargo_group":group,"cause":outcome.cause}),
                grant_id:g.id.clone(),actor:actor.into(),outcome:outcome.status.clone(),recovery_location:outcome.recovery_location.clone(),
                measured_free_space_delta:None,observed_path_state:Some("see recovery manifest and outcome".into()),recorded_at:at,
            })?;
            outcomes.push(outcome);
            continue;
        }
        let Ok(meta) = fs_gate::symlink_metadata(&unit.path) else {
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
            crate::git::linked_common_dir(&unit.path)
        } else {
            None
        };
        match crate::recheck::newest_mtime(&unit.path, 2_000_000) {
            None => {
                outcome.cause = Some("could not re-observe the tree before acting".into());
                outcomes.push(outcome);
                continue;
            }
            // Strictly `>`, and the same-second case is *not* this
            // gate's job.
            //
            // The 2026-09-22 re-review's CE2 is a rewrite inside the
            // same whole second as plan creation, which `>` reads as
            // "nothing changed". The first repair made this `>=`, and
            // that over-refuses catastrophically: `plan.created_at` is
            // whole seconds, so an ordinary propose-then-execute on a
            // tree written moments earlier -- the normal case --
            // refused itself. Two existing tests caught it
            // (`evidence_action_recheck.rs`), and a user would have
            // caught it the first time they used the tool.
            //
            // The review named the alternative and it is the right one:
            // resolve the same second where the resolution exists. Every
            // member carries `recheck::ReviewedMember`'s nanosecond
            // fingerprint (`mtime_ns`, `ctime_ns`, inode, len), and the
            // recheck below compares it before anything is moved, so a
            // same-second in-place rewrite is caught there -- precisely,
            // rather than by refusing every plan that shares a second
            // with its own tree.
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
        // The shared live-state recheck, on the ordinary filesystem path
        // too (`.oh/guardrails/execution-sinks-recheck-live-state.md`).
        //
        // Two findings from the PR #123 review are fixed here together.
        // The recheck this replaces refused only on `Known(Bool(true))`,
        // so a permission-denied or unavailable occupancy answer fell
        // through and *authorized* the removal -- a fail-open gate
        // replacing a fail-closed one. And human keep/protect intent was
        // enforced at proposal only, so `swamp protect` added after
        // approval did not stop an ordinary artifact row being moved.
        let proof = match crate::recheck::run_all(dir, &unit.path, unit.reviewed.as_ref(), &[]) {
            Ok(proof) => proof,
            Err(e) => {
                outcome.cause = Some(e.to_string());
                outcomes.push(outcome);
                continue;
            }
        };
        if keep_executables && unit.verb == "delete" {
            match preserve_executables(&unit.path, &unit.worktree_path, &auth) {
                Ok(kept) => outcome.preserved = kept.into_iter().map(|k| k.to).collect(),
                Err(e) => {
                    outcome.status = "failed".into();
                    outcome.cause = Some(format!("could not preserve executables: {e}"));
                    outcomes.push(outcome);
                    continue;
                }
            }
        }
        let dest_name = format!("{}-{}-{}", basename, unit.project.replace('/', "_"), at);
        match fs_gate::destroy::trash_move(proof, &auth, trash, &dest_name) {
            Ok(dest) => {
                outcome.status = "completed".into();
                outcome.recovery_location = Some(dest);
                trashed += unit.bytes;
                spent_by_grant.insert(gi, (spent + unit.bytes, used + 1));
                if let Some(c) = &linked_common {
                    let _ = fs_gate::destroy::git_worktree_prune(
                        &auth,
                        &unit.path,
                        c.parent().unwrap_or(c),
                    );
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
        // The observed free-space change as a sourced fact, not just a
        // bare signed integer: `Statvfs` provenance, an explicit
        // `Unknown` when either reading was unavailable (never a
        // fabricated zero), and the note that names why the number can
        // legitimately differ from what was planned.
        evidence: vec![crate::reclaimability::observed_free_space_change(
            free_before,
            free_after,
        )],
        actor: actor.to_string(),
    })
}

#[cfg(test)]
mod agent_partial_removal_tests {
    //! #101's "account for partial failure (some members moved, then
    //! failure) with explicit outcome and recovery manifest": these
    //! tests call the private `execute_agent_session_removal` directly
    //! (same crate, same file) because forcing a *specific* member's
    //! `rename` to fail deterministically needs to reach in past the
    //! public `execute` surface. No real user data anywhere -- every
    //! path here is a synthetic fixture under a `tempfile::tempdir`.
    use super::*;
    use crate::agents::claude_code::{CLAUDE_CODE_TOOL_ID, identify};
    use std::fs;

    fn touch(path: &Path, content: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    /// A Claude Code session with several members (transcript,
    /// subagents-companion dir, file-history dir, todos file) -- the
    /// same fixture shape `claude_code`'s own tests use, reused here so
    /// a real, multi-member session removal is exercised, not a
    /// hand-built `AgentUnit`.
    fn fixture_session(home: &Path) -> (PathBuf, String) {
        let repo = home.join("fixture-repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        let session_id = "22222222-2222-4222-8222-222222222222".to_string();
        let proj_dir = home.join("projects").join("-fixture-repo-encoded");
        let jsonl = proj_dir.join(format!("{session_id}.jsonl"));
        let line = format!(
            "{{\"type\":\"user\",\"sessionId\":\"s\",\"cwd\":\"{}\",\"gitBranch\":\"main\"}}\n",
            repo.display()
        );
        touch(&jsonl, line.as_bytes());
        touch(
            &proj_dir.join(&session_id).join("subagents").join("a.jsonl"),
            line.as_bytes(),
        );
        touch(
            &home.join("file-history").join(&session_id).join("snap.txt"),
            b"recoverable-fixture-content",
        );
        touch(
            &home
                .join("todos")
                .join(format!("{session_id}-agent-1.json")),
            b"[]",
        );
        (jsonl, session_id)
    }

    /// Forces the *last* member (in the same sorted order
    /// `execute_agent_session_removal` itself uses) to fail its rename
    /// by pre-occupying its exact destination inside the envelope with
    /// an incompatible entry (a plain file where a directory needs to
    /// land, or vice versa -- both are reliable, portable `rename`
    /// failures). Returns the envelope path and the members in the
    /// order the function will process them, so the test can assert
    /// precisely which ones must have moved and which must not have.
    fn force_last_member_rename_to_fail(
        home: &Path,
        trash: &Path,
        session_path: &Path,
        session_id: &str,
        at: u64,
    ) -> (PathBuf, Vec<PathBuf>) {
        let cache = crate::agents::IdentificationCache::disabled();
        let candidates = identify(home, &crate::agents::IdentifyCtx::new(at, &cache));
        let current = candidates
            .iter()
            .find(|c| c.path == session_path)
            .expect("session identified");
        let mut members: Vec<PathBuf> = current.members().iter().map(|m| m.path.clone()).collect();
        members.sort();
        assert!(
            members.len() >= 2,
            "need at least two members to prove a *partial* failure, got {members:?}"
        );

        let slug = session_path.file_stem().and_then(|s| s.to_str()).unwrap();
        assert_eq!(slug, session_id);
        let envelope = trash.join(format!("agent-session-{slug}-{at}"));
        fs::create_dir_all(&envelope).unwrap();

        let last_idx = members.len() - 1;
        let last = &members[last_idx];
        let name = last.file_name().and_then(|n| n.to_str()).unwrap();
        let dest = envelope.join(format!("{last_idx}-{name}"));
        let last_is_dir = fs::symlink_metadata(last).unwrap().is_dir();
        if last_is_dir {
            // Renaming a directory onto an existing non-directory path
            // fails (ENOTDIR) on every platform this project targets.
            fs::write(&dest, b"blocking").unwrap();
        } else {
            // Renaming a file onto an existing non-empty directory
            // fails (EISDIR/ENOTEMPTY) on every platform this project
            // targets.
            fs::create_dir_all(dest.join("blocking-child")).unwrap();
        }
        (envelope, members)
    }

    #[test]
    fn partial_session_removal_reports_the_envelope_and_writes_a_restore_manifest() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let trash = tempfile::tempdir().unwrap();
        let (session_path, session_id) = fixture_session(home);
        let at = 5_000_000u64;
        let (envelope, members) =
            force_last_member_rename_to_fail(home, trash.path(), &session_path, &session_id, at);

        let meta = AgentPlanMeta {
            tool_id: CLAUDE_CODE_TOOL_ID.to_string(),
            tool_home: home.to_path_buf(),
            category: "sessions".to_string(),
            session_members: Some(members.clone()),
        };
        let store = tempfile::tempdir().unwrap();
        let reviewed = crate::recheck::capture(&session_path).ok();
        let err = execute_agent_session_removal(
            store.path(),
            &meta,
            &session_path,
            &members,
            reviewed.as_ref(),
            trash.path(),
            at,
            &crate::authority::for_tests(std::slice::from_ref(&session_path)),
        )
        .expect_err("the last member's rename was deliberately blocked");
        let partial = err
            .downcast_ref::<PartialAgentRemoval>()
            .unwrap_or_else(|| panic!("expected PartialAgentRemoval, got: {err:#}"));
        assert_eq!(partial.envelope, envelope);
        assert!(
            partial.moved_bytes > 0 || members.len() == 1,
            "at least the members before the blocked one must have moved"
        );

        // The recovery manifest exists and reflects exactly what
        // happened: every member but the last is "moved" (and no
        // longer at its original path); the last is "pending" (and
        // untouched at its original path).
        let manifest_bytes = fs::read(envelope.join("restore.json")).expect("restore.json exists");
        let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
        assert_eq!(manifest["tool_id"], CLAUDE_CODE_TOOL_ID);
        let manifest_members = manifest["members"].as_array().unwrap();
        assert_eq!(manifest_members.len(), members.len());
        let last_idx = members.len() - 1;
        for (i, m) in manifest_members.iter().enumerate() {
            let original = std::path::PathBuf::from(m["original"].as_str().unwrap());
            assert_eq!(&original, &members[i]);
            if i == last_idx {
                assert_eq!(m["status"], "pending", "{manifest}");
                assert!(
                    original.exists(),
                    "the blocked member must remain at its original location"
                );
            } else {
                assert_eq!(m["status"], "moved", "{manifest}");
                assert!(
                    !original.exists(),
                    "a member reported \"moved\" must no longer be at its original location"
                );
                assert!(m["moved_to"].as_str().is_some());
            }
        }

        // Nothing here ever wrote the fixture's own content into the
        // manifest (metadata only: original path, byte count, status).
        assert!(
            !manifest_bytes
                .windows(b"recoverable-fixture-content".len())
                .any(|w| w == b"recoverable-fixture-content")
        );
    }
}
