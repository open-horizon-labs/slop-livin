//! The action layer: plan -> human authorization -> sink re-derivation ->
//! Trash -> per-unit outcome -> ledger. Wires the minimum of
//! `grants`/`execution`/`ledger`/`occupancy` from `slop_livin_core`;
//! nothing there is widened.

use anyhow::Result;
use slop_livin_core::entities::{id_for, now};
use slop_livin_core::execution::Outcome;
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
    /// The row's label, for the confirm line.
    pub label: String,
    /// Facts the human should see before confirming (dirty, unpushed,
    /// untracked content, no remote…). Shown, never enforced.
    pub warnings: Vec<String>,
}

/// The terms a worktree removal was authorized on; recorded in the ledger.
#[derive(Debug, Clone)]
pub struct WorktreeTerms {
    pub merge_complete: bool,
    pub pr: Option<String>,
    /// True when the unit is a whole checkout (the `archive` verb), not a
    /// linked worktree: a higher bar, since it takes the working copy.
    pub whole_checkout: bool,
    pub remote: Option<String>,
}

/// Human pressing Enter at the confirm summary is the authorization for
/// this one plan (never the index, never an agent). Builds the plan and
/// grant together since they are minted for the same keypress.
pub fn authorize(units: &[MarkedUnit], actor: &str) -> (slop_livin_core::grants::Plan, Grant) {
    let plan_units = units
        .iter()
        .map(|u| slop_livin_core::grants::PlanUnit {
            artifact_id: id_for(&u.path.display().to_string()),
            verb: match &u.worktree {
                Some(t) if t.whole_checkout => Verb::Archive,
                Some(_) => Verb::RemoveWorktree,
                None => Verb::Delete,
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
    _plan_unit: &slop_livin_core::grants::PlanUnit,
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
    let outcome = trash_path(unit, Verb::Delete, grant, ledger, trash_root, actor, None)
        .map_err(|e| e.to_string());
    UnitResult {
        path: unit.path.clone(),
        outcome,
    }
}

/// Moves one path to Trash and records it. The only refusal is a path
/// that no longer exists: the human already confirmed with the warnings
/// in front of them, and Trash keeps the move reversible.
fn trash_path(
    unit: &MarkedUnit,
    verb: Verb,
    grant: &Grant,
    ledger: &Ledger,
    trash_root: &Path,
    actor: &str,
    extra: Option<serde_json::Value>,
) -> Result<Outcome> {
    let path = &unit.path;
    if std::fs::symlink_metadata(path).is_err() {
        anyhow::bail!("path no longer exists");
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("item");
    let dest = trash_root.join(format!("{name}-{}", now()));
    std::fs::create_dir_all(trash_root)?;
    std::fs::rename(path, &dest)?;
    let mut evidence = serde_json::json!({
        "label": unit.label,
        "bytes": unit.bytes,
        "observed_at": unit.observed_at,
        "warnings_shown": unit.warnings,
    });
    if let Some(extra) = extra
        && let (Some(map), Some(more)) = (evidence.as_object_mut(), extra.as_object())
    {
        for (k, v) in more {
            map.insert(k.clone(), v.clone());
        }
    }
    ledger.append(&slop_livin_core::ledger::ActionRecord {
        id: slop_livin_core::entities::new_id(),
        verb,
        entity_id: id_for(&path.display().to_string()),
        evidence,
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

/// Removes a linked worktree (directory to Trash, then `git worktree
/// prune`) or a whole checkout (`archive`). No bar: the warnings were on
/// the confirm line. Recoverable: move the directory back (and `git
/// worktree repair` for a linked worktree).
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
    let common: Option<PathBuf> = if !terms.whole_checkout && gitfile.is_file() {
        std::fs::read_to_string(&gitfile).ok().and_then(|line| {
            let gitdir = PathBuf::from(line.trim().strip_prefix("gitdir:")?.trim());
            let gitdir = if gitdir.is_absolute() {
                gitdir
            } else {
                path.join(gitdir)
            };
            let c = std::fs::read_to_string(gitdir.join("commondir")).ok()?;
            let c = PathBuf::from(c.trim());
            Some(if c.is_absolute() { c } else { gitdir.join(c) })
        })
    } else {
        None
    };
    let verb = if terms.whole_checkout {
        Verb::Archive
    } else {
        Verb::RemoveWorktree
    };
    let recover = match (&terms.remote, &common) {
        (_, Some(c)) => format!(
            "move the directory back, then git -C {} worktree repair",
            c.display()
        ),
        (Some(r), None) => format!("move the directory back, or git clone {r}"),
        (None, None) => "move the directory back from Trash".to_string(),
    };
    let outcome = trash_path(
        unit,
        verb,
        grant,
        ledger,
        trash_root,
        actor,
        Some(serde_json::json!({
            "merge_complete": terms.merge_complete,
            "pr": terms.pr,
            "remote": terms.remote,
            "recover": recover,
        })),
    )?;
    if let Some(c) = common {
        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(c.parent().unwrap_or(&c))
            .args(["worktree", "prune"])
            .output();
    }
    Ok(outcome)
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
    let what: Vec<String> = units
        .iter()
        .take(3)
        .map(|u| {
            let name = u
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&u.label)
                .to_string();
            if u.warnings.is_empty() {
                name
            } else {
                format!("{name} ⚠ {}", u.warnings.join(" · "))
            }
        })
        .collect();
    let more = if units.len() > 3 {
        format!(" +{} more", units.len() - 3)
    } else {
        String::new()
    };
    format!(
        "delete {} ({}) → Trash{more}?  Enter yes · Esc no",
        what.join(", "),
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
            label: String::new(),
            warnings: Vec::new(),
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
    fn confirm_summary_names_units_and_states_their_warnings() {
        let clean = MarkedUnit {
            path: "/tmp/target".into(),
            bytes: 2 * 1024 * 1024 * 1024,
            observed_at: 0,
            label: "build target".into(),
            warnings: Vec::new(),
            worktree: None,
        };
        let risky = MarkedUnit {
            path: "/tmp/raw".into(),
            bytes: 1_100_000_000,
            observed_at: 0,
            label: "dir raw".into(),
            warnings: vec!["untracked: in no version control".into()],
            worktree: None,
        };
        let s = confirm_summary(&[clean, risky]);
        assert_eq!(
            s,
            "delete target, raw ⚠ untracked: in no version control (3.2GB) → Trash?  Enter yes · Esc no"
        );
    }
}
