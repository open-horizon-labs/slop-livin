//! The action layer: plan -> human authorization -> sink re-derivation ->
//! Trash -> per-unit outcome -> ledger. Wires the minimum of
//! `grants`/`execution`/`ledger`/`occupancy` from `slop_livin_core`;
//! nothing there is widened.

use anyhow::Result;
use slop_livin_core::entities::{
    ArtifactKind as EntityKind, Confidence, FactMeta, RecoveryContract, id_for, now,
};
use slop_livin_core::execution::{Outcome, execute_delete};
use slop_livin_core::grants::{Grant, Predicate, Verb, plan};
use slop_livin_core::ledger::Ledger;
use std::path::{Path, PathBuf};

use crate::model::human_bytes;

/// One unit the human marked for deletion: enough facts to re-derive the
/// entity the core action layer expects, without a second data path (the
/// facts all came from the one `Report`).
#[derive(Debug, Clone)]
pub struct MarkedUnit {
    pub path: PathBuf,
    pub bytes: u64,
    pub observed_at: u64,
    /// Set when the unit is a linked worktree rather than an artifact dir.
    pub worktree: Option<WorktreeTerms>,
}

/// The terms a worktree removal was authorized on; recorded in the ledger.
#[derive(Debug, Clone)]
pub struct WorktreeTerms {
    pub merge_complete: bool,
    pub pr: Option<String>,
}

/// Human pressing Enter at the confirm summary is the authorization for
/// this one plan (never the index, never an agent). Builds the plan and
/// grant together since they are minted for the same keypress.
pub fn authorize(units: &[MarkedUnit], actor: &str) -> (slop_livin_core::grants::Plan, Grant) {
    let plan_units = units
        .iter()
        .map(|u| slop_livin_core::grants::PlanUnit {
            artifact_id: id_for(&u.path.display().to_string()),
            verb: if u.worktree.is_some() {
                Verb::RemoveWorktree
            } else {
                Verb::Delete
            },
            expected_bytes: u.bytes,
            evidence_observed_at: u.observed_at,
            undo_cost: "trash".into(),
        })
        .collect();
    let plan = plan(plan_units, now() + 300);
    let grant = Grant {
        id: slop_livin_core::entities::new_id(),
        verb: Verb::Delete,
        predicate: Predicate {
            project_id: None,
            kind: None,
            max_bytes: 0,
            require_fresh_within_secs: 3600,
        },
        scope: plan.units.iter().map(|u| u.artifact_id.clone()).collect(),
        expires_at: plan.expires_at,
        actor: actor.to_string(),
        created_outside_index: true,
    };
    (plan, grant)
}

/// Result of executing one marked unit, for the inline per-unit outcome.
#[derive(Debug, Clone)]
pub struct UnitResult {
    pub path: PathBuf,
    pub outcome: Result<Outcome, String>,
}

/// Sink re-derivation happens inside `execute_delete` itself (it
/// re-`symlink_metadata`s the path and checks occupancy immediately
/// before the rename); this just drives one unit through it and records
/// the per-unit result.
fn execute_one(
    unit: &MarkedUnit,
    plan_unit: &slop_livin_core::grants::PlanUnit,
    grant: &Grant,
    ledger: &Ledger,
    trash_root: &Path,
    actor: &str,
) -> UnitResult {
    if let Some(terms) = &unit.worktree {
        return UnitResult {
            path: unit.path.clone(),
            outcome: remove_worktree(unit, terms, grant, ledger, trash_root, actor)
                .map_err(|e| e.to_string()),
        };
    }
    let artifact = slop_livin_core::entities::Artifact {
        id: plan_unit.artifact_id.clone(),
        project_id: None,
        kind: EntityKind::Unknown,
        path: unit.path.clone(),
        relative_path: None,
        bytes: unit.bytes,
        recovery: RecoveryContract::LocalRebuild,
        present: true,
        regrowth_count: 0,
        meta: FactMeta {
            observed_at: unit.observed_at,
            source: "tui".into(),
            confidence: Confidence::High,
            horizon_exceeded: false,
        },
    };
    let outcome = execute_delete(&artifact, plan_unit, grant, ledger, trash_root, actor)
        .map_err(|e| e.to_string());
    UnitResult {
        path: unit.path.clone(),
        outcome,
    }
}

/// Removes a linked worktree: re-derives the terms at the sink (still a
/// linked worktree, clean, nothing unpushed, unlocked, unoccupied), moves
/// the directory to Trash, then `git worktree prune` in the main repo so
/// git forgets the now-missing checkout. Recoverable: move the directory
/// back and run `git worktree repair`.
fn remove_worktree(
    unit: &MarkedUnit,
    terms: &WorktreeTerms,
    grant: &Grant,
    ledger: &Ledger,
    trash_root: &Path,
    actor: &str,
) -> Result<Outcome> {
    let path = &unit.path;
    let gitfile = path.join(".git");
    let meta = std::fs::symlink_metadata(&gitfile)
        .map_err(|_| anyhow::anyhow!("not a worktree: no .git here"))?;
    if !meta.is_file() {
        anyhow::bail!("main checkout — not removable as a worktree");
    }
    let gitdir_line = std::fs::read_to_string(&gitfile)?;
    let gitdir = gitdir_line
        .trim()
        .strip_prefix("gitdir:")
        .map(|s| PathBuf::from(s.trim()))
        .ok_or_else(|| anyhow::anyhow!("unreadable .git file"))?;
    let gitdir = if gitdir.is_absolute() {
        gitdir
    } else {
        path.join(gitdir)
    };
    let common = std::fs::read_to_string(gitdir.join("commondir"))
        .map(|c| {
            let c = PathBuf::from(c.trim());
            if c.is_absolute() { c } else { gitdir.join(c) }
        })
        .map_err(|_| anyhow::anyhow!("not a linked worktree (no commondir)"))?;
    // Sink re-derivation of the terms, live.
    let (_, raw) = slop_livin_core::signals::compute_signals_raw(path, now());
    match raw.dirty {
        Some(false) => {}
        Some(true) => anyhow::bail!("dirty: uncommitted changes"),
        None => anyhow::bail!("could not determine dirty state"),
    }
    match raw.unpushed {
        Some(0) => {}
        Some(n) => anyhow::bail!("{n} unpushed commit{}", if n == 1 { "" } else { "s" }),
        None => anyhow::bail!("unpushed count unknown (no upstream)"),
    }
    if raw.locked != Some(false) {
        anyhow::bail!("worktree is locked");
    }
    if slop_livin_core::occupancy::occupied(path) {
        anyhow::bail!("occupied: a process holds files under this worktree");
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("worktree");
    let dest = trash_root.join(format!("worktree-{name}-{}", now()));
    std::fs::create_dir_all(trash_root)?;
    std::fs::rename(path, &dest)?;
    let prune = std::process::Command::new("git")
        .arg("-C")
        .arg(common.parent().unwrap_or(&common))
        .args(["worktree", "prune"])
        .output();
    let pruned = prune.as_ref().map(|o| o.status.success()).unwrap_or(false);
    ledger.append(&slop_livin_core::ledger::ActionRecord {
        id: slop_livin_core::entities::new_id(),
        verb: Verb::RemoveWorktree,
        entity_id: id_for(&path.display().to_string()),
        evidence: serde_json::json!({
            "bytes": unit.bytes,
            "observed_at": unit.observed_at,
            "terms": {"clean": true, "unpushed": 0, "unlocked": true,
                       "merge_complete": terms.merge_complete, "pr": terms.pr},
            "git_worktree_prune": pruned,
            "recover": format!("mv {} {} && git -C {} worktree repair", dest.display(), path.display(), common.display()),
        }),
        grant_id: grant.id.clone(),
        actor: actor.into(),
        outcome: "completed".into(),
        recovery_location: Some(dest.clone()),
        measured_free_space_delta: None,
        observed_path_state: Some("trashed".into()),
        recorded_at: now(),
    })?;
    Ok(Outcome {
        unit_id: id_for(&path.display().to_string()),
        status: "completed".into(),
        reason: None,
        intended_bytes: unit.bytes,
        observed_free_space_delta: None,
    })
}

/// Executes every unit in the plan in order, returning one result per
/// unit. A failure on one unit does not stop the rest -- the footer
/// reports refusals per unit, not as a single aborted batch.
pub fn execute_plan(
    units: &[MarkedUnit],
    plan: &slop_livin_core::grants::Plan,
    grant: &Grant,
    ledger: &Ledger,
    trash_root: &Path,
    actor: &str,
) -> Vec<UnitResult> {
    units
        .iter()
        .zip(plan.units.iter())
        .map(|(u, pu)| execute_one(u, pu, grant, ledger, trash_root, actor))
        .collect()
}

/// The default Trash root: `~/.Trash` on macOS. Overridable via
/// `SLOP_LIVIN_TRASH_DIR` for tests and CI, which never wants a real
/// `~/.Trash`.
pub fn trash_root() -> PathBuf {
    if let Ok(dir) = std::env::var("SLOP_LIVIN_TRASH_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".Trash")
}

/// Free space on the volume containing `path`, in bytes, via `df -k`.
/// Returns `None` if `df` cannot be read (kept read-only/advisory: a
/// missing measurement never blocks or fakes the reported result).
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
    let line = text.lines().nth(1)?;
    let fields: Vec<&str> = line.split_whitespace().collect();
    // macOS `df -k`: Filesystem 1024-blocks Used Available Capacity ...
    let available_kb: u64 = fields.get(3)?.parse().ok()?;
    Some(available_kb * 1024)
}

/// Human summary line for the confirm banner: `delete 3 units · 1.9 GB
/// -> Trash · Enter confirm · Esc cancel`.
pub fn confirm_summary(units: &[MarkedUnit]) -> String {
    let total: u64 = units.iter().map(|u| u.bytes).sum();
    let wts = units.iter().filter(|u| u.worktree.is_some()).count();
    let arts = units.len() - wts;
    let mut what = Vec::new();
    if arts > 0 {
        what.push(format!(
            "delete {arts} artifact{}",
            if arts == 1 { "" } else { "s" }
        ));
    }
    if wts > 0 {
        what.push(format!(
            "remove {wts} worktree{}",
            if wts == 1 { "" } else { "s" }
        ));
    }
    format!(
        "{} · {} → Trash · Enter confirm · Esc cancel",
        what.join(" + "),
        human_bytes(total)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn end_to_end_delete_moves_to_trash_and_appends_ledger() {
        let workdir = tempdir().unwrap();
        let target = workdir.path().join("node_modules");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("marker"), b"x").unwrap();
        let bytes = 1024u64;

        let unit = MarkedUnit {
            path: target.clone(),
            bytes,
            observed_at: now(),
            worktree: None,
        };
        let (plan, grant) = authorize(std::slice::from_ref(&unit), "human");
        let ledger = Ledger::open(workdir.path().join("ledger.jsonl")).unwrap();
        let trash = workdir.path().join("Trash");
        let results = execute_plan(&[unit], &plan, &grant, &ledger, &trash, "human");

        assert_eq!(results.len(), 1);
        assert!(results[0].outcome.is_ok(), "{:?}", results[0].outcome);
        assert!(!target.exists());
        let entries: Vec<_> = std::fs::read_dir(&trash).unwrap().collect();
        assert_eq!(entries.len(), 1, "node_modules should have landed in Trash");

        let records = ledger.all().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].outcome, "completed");
        assert_eq!(records[0].actor, "human");
    }

    #[test]
    fn confirm_summary_pluralizes() {
        let u = MarkedUnit {
            path: "/tmp/a".into(),
            bytes: 2 * 1024 * 1024 * 1024,
            observed_at: 0,
            worktree: None,
        };
        assert_eq!(
            confirm_summary(&[u.clone(), u]),
            "delete 2 artifacts · 4.3GB → Trash · Enter confirm · Esc cancel"
        );
    }
}
