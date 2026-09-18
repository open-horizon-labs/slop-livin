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
}

/// Human pressing Enter at the confirm summary is the authorization for
/// this one plan (never the index, never an agent). Builds the plan and
/// grant together since they are minted for the same keypress.
pub fn authorize(units: &[MarkedUnit], actor: &str) -> (slop_livin_core::grants::Plan, Grant) {
    let plan_units = units
        .iter()
        .map(|u| slop_livin_core::grants::PlanUnit {
            artifact_id: id_for(&u.path.display().to_string()),
            verb: Verb::Delete,
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
    format!(
        "delete {} unit{} · {} → Trash · Enter confirm · Esc cancel",
        units.len(),
        if units.len() == 1 { "" } else { "s" },
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
        };
        assert_eq!(
            confirm_summary(&[u.clone(), u]),
            "delete 2 units · 4.0GB → Trash · Enter confirm · Esc cancel"
        );
    }
}
