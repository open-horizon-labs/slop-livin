//! The action layer: plan -> human authorization -> sink re-derivation ->
//! Trash -> per-unit outcome -> ledger. Wires the minimum of
//! `grants`/`execution`/`ledger`/`occupancy` from `swamp_core`;
//! nothing there is widened.

use anyhow::Result;
use std::path::{Path, PathBuf};
use swamp_core::entities::{id_for, now};
use swamp_core::execution::Outcome;
use swamp_core::grants::{Grant, Predicate, Verb, plan};
use swamp_core::authority::{Authorized, HumanConfirmed};
use swamp_core::ledger::Ledger;

use crate::model::human_bytes;

/// One unit the human marked for deletion: enough facts to re-derive the
/// entity the core action layer expects, without a second data path (the
/// facts all came from the one `Report`).
#[derive(Debug, Clone)]
pub struct MarkedUnit {
    pub cargo_plan: Option<swamp_core::actions::Plan>,
    /// The agent-storage counterpart of `cargo_plan` (#101's Agents-view
    /// TUI wiring): a plan `swamp_core::actions::propose_agents` already
    /// built and refused-or-cleared at mark time. Kept as a separate
    /// field, not folded into `cargo_plan`, because `execute_one`'s
    /// cargo path bails when `keep_executables` is set -- a
    /// build-artifact-only concern that must never refuse an unrelated
    /// agent-storage removal just because the human also has "keep
    /// executables" toggled on.
    pub agent_plan: Option<swamp_core::actions::Plan>,
    pub path: PathBuf,
    /// Set for a Docker object: what removing it actually runs, and the
    /// fact that it never reaches Trash.
    pub docker: Option<swamp_core::docker::Removal>,
    /// The worktree containing the unit (itself, for a worktree row).
    pub worktree_path: PathBuf,
    pub bytes: u64,
    pub observed_at: u64,
    /// Set when the unit is a linked worktree rather than an artifact dir.
    pub worktree: Option<WorktreeTerms>,
    /// The row's label, for the confirm line.
    pub label: String,
    /// Facts the human should see before confirming (dirty, unpushed,
    /// untracked content, no remote…). Shown, never enforced.
    pub warnings: Vec<String>,
    /// What was at `path` when the human marked it, so the sink can
    /// refuse a unit that changed between marking and confirming.
    ///
    /// The hardened `execution_sinks_recheck_live_state` audit found
    /// this missing on 2026-09-22: `trash_path` is the TUI's live delete
    /// path, it moved user data after only a `symlink_metadata`
    /// existence check, and its own doc comment pointed at
    /// `core::execution::execute_delete` -- a function with no callers
    /// at all. The hand-written sink-file list never covered
    /// `crates/tui/src/actions.rs`, so nothing saw it.
    ///
    /// `None` is itself a refusal at the sink, exactly as it is in
    /// `recheck::reviewed_snapshot`.
    pub reviewed: Option<swamp_core::recheck::ReviewedIdentity>,
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
/// grant together since they are minted for the same keypress, from the
/// [`HumanConfirmed`] the confirm dialog (`app.rs`, the one reviewed TUI
/// confirmation site) minted.
pub fn authorize(
    units: &[MarkedUnit],
    confirmed: &HumanConfirmed,
) -> (swamp_core::grants::Plan, Grant) {
    let actor = confirmed.actor();
    let plan_units = units
        .iter()
        .map(|u| swamp_core::grants::PlanUnit {
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
        id: swamp_core::entities::new_id(),
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
    _plan_unit: &swamp_core::grants::PlanUnit,
    grant: &Grant,
    confirmed: &HumanConfirmed,
    ledger: &Ledger,
    trash_root: &Path,
    keep_executables: bool,
) -> UnitResult {
    let actor = confirmed.actor();
    if let Some(plan) = &unit.cargo_plan {
        let result = (|| -> Result<Outcome> {
            if !grant.created_outside_index
                || grant.expires_at < now()
                || !grant
                    .scope
                    .contains(&id_for(&unit.path.display().to_string()))
                || _plan_unit.artifact_id != id_for(&unit.path.display().to_string())
            {
                anyhow::bail!("Cargo selection not covered by current confirmation");
            }
            if keep_executables {
                anyhow::bail!("keep-executables conflicts with selective build removal");
            }
            let store = ledger
                .path()
                .parent()
                .ok_or_else(|| anyhow::anyhow!("ledger has no store directory"))?;
            if swamp_core::actions::load_plan(store, &plan.id)
                .is_ok_and(|p| p.status == swamp_core::actions::PlanStatus::Executed)
            {
                anyhow::bail!("Cargo plan already executed");
            }
            swamp_core::actions::save_plan(store, plan)?;
            swamp_core::actions::approve_confirmed(store, &plan.id, confirmed)?;
            let result =
                swamp_core::actions::execute_with_trash(store, &plan.id, actor, trash_root)?;
            let outcome = result
                .outcomes
                .first()
                .ok_or_else(|| anyhow::anyhow!("{}", result.state))?;
            if outcome.status != "completed" {
                anyhow::bail!(
                    "{}",
                    outcome.cause.as_deref().unwrap_or("Cargo action refused")
                );
            }
            Ok(Outcome {
                unit_id: id_for(&unit.path.display().to_string()),
                status: "completed".into(),
                reason: None,
                intended_bytes: unit.bytes,
                observed_free_space_delta: result.freed_measured,
            })
        })();
        return UnitResult {
            path: unit.path.clone(),
            outcome: result.map_err(|e| e.to_string()),
        };
    }
    if let Some(plan) = &unit.agent_plan {
        let result = (|| -> Result<Outcome> {
            if !grant.created_outside_index
                || grant.expires_at < now()
                || !grant
                    .scope
                    .contains(&id_for(&unit.path.display().to_string()))
                || _plan_unit.artifact_id != id_for(&unit.path.display().to_string())
            {
                anyhow::bail!("agent-storage selection not covered by current confirmation");
            }
            let store = ledger
                .path()
                .parent()
                .ok_or_else(|| anyhow::anyhow!("ledger has no store directory"))?;
            if swamp_core::actions::load_plan(store, &plan.id)
                .is_ok_and(|p| p.status == swamp_core::actions::PlanStatus::Executed)
            {
                anyhow::bail!("agent-storage plan already executed");
            }
            swamp_core::actions::save_plan(store, plan)?;
            swamp_core::actions::approve_confirmed(store, &plan.id, confirmed)?;
            let result =
                swamp_core::actions::execute_with_trash(store, &plan.id, actor, trash_root)?;
            let outcome = result
                .outcomes
                .first()
                .ok_or_else(|| anyhow::anyhow!("{}", result.state))?;
            if outcome.status != "completed" {
                anyhow::bail!(
                    "{}",
                    outcome
                        .cause
                        .as_deref()
                        .unwrap_or("agent-storage action refused")
                );
            }
            Ok(Outcome {
                unit_id: id_for(&unit.path.display().to_string()),
                status: "completed".into(),
                reason: None,
                intended_bytes: unit.bytes,
                observed_free_space_delta: result.freed_measured,
            })
        })();
        return UnitResult {
            path: unit.path.clone(),
            outcome: result.map_err(|e| e.to_string()),
        };
    }
    // What the human confirmed: exactly this unit, under this dialog's
    // grant. Every destructive step below borrows it.
    let Some(auth) =
        swamp_core::authority::authorize_confirmed(confirmed, &grant.id, &[unit.path.clone()])
    else {
        return UnitResult {
            path: unit.path.clone(),
            outcome: Err("not covered by the confirmation".into()),
        };
    };
    if let Some(terms) = &unit.worktree {
        return UnitResult {
            path: unit.path.clone(),
            outcome: remove_worktree(unit, terms, grant, &auth, ledger, trash_root, actor)
                .map_err(|e| e.to_string()),
        };
    }
    // A Docker object is not a path: it is removed through the daemon,
    // permanently, and the ledger records that there is no recovery
    // location rather than pretending there is one.
    if let Some(target) = &unit.docker {
        return UnitResult {
            path: unit.path.clone(),
            outcome: remove_docker(unit, target, grant, &auth, ledger, actor)
                .map_err(|e| e.to_string()),
        };
    }
    // Compiled outputs first, so a failure to copy them refuses the unit
    // before anything moves.
    let extra = if keep_executables {
        match swamp_core::actions::preserve_executables(&unit.path, &unit.worktree_path, &auth) {
            Ok(kept) => Some(serde_json::json!({
                "preserved": kept.iter().map(|k| k.to.display().to_string()).collect::<Vec<_>>()
            })),
            Err(e) => {
                return UnitResult {
                    path: unit.path.clone(),
                    outcome: Err(format!("could not preserve executables: {e}")),
                };
            }
        }
    } else {
        None
    };
    let outcome = trash_path(unit, Verb::Delete, grant, &auth, ledger, trash_root, actor, extra)
        .map_err(|e| e.to_string());
    UnitResult {
        path: unit.path.clone(),
        outcome,
    }
}

/// Removes one Docker object. Re-derived at the sink (still present),
/// then handed to the daemon, whose own refusal text is the outcome when
/// it declines — an image a container still references, a volume still
/// mounted. Nothing here is reversible, so `recovery_location` is `None`
/// and the ledger says so.
fn remove_docker(
    unit: &MarkedUnit,
    target: &swamp_core::docker::Removal,
    grant: &Grant,
    auth: &Authorized,
    ledger: &Ledger,
    actor: &str,
) -> Result<Outcome> {
    swamp_core::docker::still_removable(target).map_err(|e| anyhow::anyhow!(e))?;
    swamp_core::docker::remove(target, auth, &unit.path).map_err(|e| anyhow::anyhow!(e))?;
    ledger.append(&swamp_core::ledger::ActionRecord {
        id: swamp_core::entities::new_id(),
        verb: Verb::Delete,
        entity_id: id_for(&unit.path.display().to_string()),
        evidence: serde_json::json!({
            "label": unit.label,
            "bytes": unit.bytes,
            "observed_at": unit.observed_at,
            "warnings_shown": unit.warnings,
            "docker": format!("{target:?}"),
            "permanent": true,
        }),
        grant_id: grant.id.clone(),
        actor: actor.to_string(),
        outcome: "completed".to_string(),
        // The daemon has no Trash: there is nowhere to point at.
        recovery_location: None,
        measured_free_space_delta: None,
        observed_path_state: Some("removed via docker".to_string()),
        recorded_at: now(),
    })?;
    Ok(Outcome {
        unit_id: unit.path.display().to_string(),
        status: "completed".to_string(),
        reason: None,
        intended_bytes: unit.bytes,
        observed_free_space_delta: None,
    })
}

/// Moves one path to Trash and records it, after the same three live
/// rechecks every other destructive sink performs
/// (`.oh/guardrails/execution-sinks-recheck-live-state.md`): the unit is
/// still the thing that was marked, no `swamp protect` entry covers it
/// in either direction (loaded fresh, never from the mark), and nothing
/// holds it open (`Unknown` refuses).
///
/// Before 2026-09-22 the only refusal here was "the path no longer
/// exists". Human confirmation authorizes removing *what was shown*; it
/// does not authorize removing whatever happens to be at that path when
/// the worker thread gets there.
#[allow(clippy::too_many_arguments)]
fn trash_path(
    unit: &MarkedUnit,
    verb: Verb,
    grant: &Grant,
    auth: &Authorized,
    ledger: &Ledger,
    trash_root: &Path,
    actor: &str,
    extra: Option<serde_json::Value>,
) -> Result<Outcome> {
    let path = &unit.path;
    if swamp_core::fs_gate::symlink_metadata(path).is_err() {
        anyhow::bail!("path no longer exists");
    }
    let store = ledger
        .path()
        .parent()
        .ok_or_else(|| anyhow::anyhow!("no store directory for the protect list"))?
        .to_path_buf();
    // Identity + membership, protection loaded fresh in both directions,
    // and tri-state occupancy of every member: all three, or no proof,
    // and without a proof there is nothing `trash_move` will take.
    let proof = swamp_core::recheck::run_all(&store, path, unit.reviewed.as_ref(), &[])?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("item");
    let dest = swamp_core::fs_gate::destroy::trash_move(
        proof,
        auth,
        trash_root,
        &format!("{name}-{}", now()),
    )?;
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
    ledger.append(&swamp_core::ledger::ActionRecord {
        id: swamp_core::entities::new_id(),
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
    auth: &Authorized,
    ledger: &Ledger,
    trash_root: &Path,
    actor: &str,
) -> Result<Outcome> {
    let path = &unit.path;
    let common: Option<PathBuf> = if terms.whole_checkout {
        None
    } else {
        swamp_core::git::linked_common_dir(path)
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
        auth,
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
        let _ = swamp_core::fs_gate::destroy::git_worktree_prune(
            auth,
            path,
            c.parent().unwrap_or(&c),
        );
    }
    Ok(outcome)
}

/// Executes every unit in the plan in order, returning one result per
/// unit. A failure on one unit does not stop the rest -- the footer
/// reports refusals per unit, not as a single aborted batch.
pub fn execute_plan(
    units: &[MarkedUnit],
    plan: &swamp_core::grants::Plan,
    grant: &Grant,
    confirmed: &HumanConfirmed,
    ledger: &Ledger,
    trash_root: &Path,
    keep_executables: bool,
) -> Vec<UnitResult> {
    execute_plan_progress(
        units,
        plan,
        grant,
        confirmed,
        ledger,
        trash_root,
        keep_executables,
        |_, _, _| true,
    )
}

/// Returning false stops before the next group, never during an in-flight move.
#[allow(clippy::too_many_arguments)]
pub fn execute_plan_progress(
    units: &[MarkedUnit],
    plan: &swamp_core::grants::Plan,
    grant: &Grant,
    confirmed: &HumanConfirmed,
    ledger: &Ledger,
    trash_root: &Path,
    keep_executables: bool,
    mut progress: impl FnMut(usize, &Path, Option<bool>) -> bool,
) -> Vec<UnitResult> {
    if units
        .iter()
        .any(|u| u.cargo_plan.is_some() || u.agent_plan.is_some())
    {
        for (i, a) in units.iter().enumerate() {
            for b in units.iter().skip(i + 1) {
                if swamp_core::scope::overlapping(&a.path, &b.path) {
                    return units
                        .iter()
                        .map(|u| UnitResult {
                            path: u.path.clone(),
                            outcome: Err(
                                "overlapping parent/child selection; nothing executed".into()
                            ),
                        })
                        .collect();
                }
            }
        }
    }
    let mut results = Vec::new();
    for (i, (u, pu)) in units.iter().zip(plan.units.iter()).enumerate() {
        if !progress(i, &u.path, None) {
            break;
        }
        let result = execute_one(u, pu, grant, confirmed, ledger, trash_root, keep_executables);
        let keep_going = progress(i + 1, &u.path, Some(result.outcome.is_ok()));
        results.push(result);
        if !keep_going {
            break;
        }
    }
    results
}

/// The default Trash root: `~/.Trash` on macOS. Overridable via
/// `SWAMP_TRASH_DIR` for tests and CI, which never wants a real
/// `~/.Trash`.
pub fn trash_root() -> PathBuf {
    if let Ok(dir) = std::env::var("SWAMP_TRASH_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".Trash")
}

/// Free space on the volume containing `path`, in bytes, via `df -k`.
/// Returns `None` if `df` cannot be read (kept read-only/advisory: a
/// missing measurement never blocks or fakes the reported result).
pub fn free_space_bytes(path: &Path) -> Option<u64> {
    swamp_core::actions::free_space_bytes(path)
}

/// Human summary line for the confirm banner: `delete 3 units · 1.9 GB
/// -> Trash · Enter confirm · Esc cancel`.
pub fn confirm_summary(units: &[MarkedUnit], keep_executables: bool) -> String {
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
    let keep = if keep_executables {
        " · keep executables → bin/ (k)"
    } else {
        " · k keep executables"
    };
    // Two destinations, and the difference is the whole point: a path
    // goes to Trash and comes back, a Docker object does not.
    let permanent_units: Vec<&MarkedUnit> = units.iter().filter(|u| u.docker.is_some()).collect();
    let permanent: u64 = permanent_units.iter().map(|u| u.bytes).sum();
    let destination = match (permanent, total - permanent) {
        (0, _) => "→ Trash".to_string(),
        (p, 0) => format!("→ removed permanently, no Trash ({})", human_bytes(p)),
        (p, t) => format!(
            "→ {} to Trash, {} removed permanently (docker, no Trash)",
            human_bytes(t),
            human_bytes(p)
        ),
    };
    // Name every unit that cannot come back, not just its bytes. One
    // project expands into many units, and the three the line has room
    // for are usually ordinary directories -- which left the one
    // irreversible thing in the plan showing as a number and nothing
    // else. The human authorizing this should read what they are
    // destroying by name.
    let no_way_back = if permanent_units.is_empty() {
        String::new()
    } else {
        let names: Vec<String> = permanent_units
            .iter()
            .take(6)
            .map(|u| {
                let what = match &u.docker {
                    Some(swamp_core::docker::Removal::Image { .. }) => "image",
                    Some(swamp_core::docker::Removal::Volume { .. }) => "volume",
                    _ => "object",
                };
                let name = if u.label.trim().is_empty() {
                    u.path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("?")
                        .to_string()
                } else {
                    u.label.trim().to_string()
                };
                format!("{name} (docker {what})")
            })
            .collect();
        let extra = permanent_units.len().saturating_sub(6);
        let extra = if extra > 0 {
            format!(" +{extra} more")
        } else {
            String::new()
        };
        format!(" · gone for good: {}{extra}", names.join(", "))
    };
    format!(
        "delete {}{more} ({}) {destination}{no_way_back}?  Enter yes · Esc no{keep}",
        what.join(", "),
        human_bytes(total)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn unit(path: &str, bytes: u64, docker: Option<swamp_core::docker::Removal>) -> MarkedUnit {
        MarkedUnit {
            cargo_plan: None,
            agent_plan: None,
            reviewed: swamp_core::recheck::capture_anchor(std::path::Path::new(path)).ok(),
            path: PathBuf::from(path),
            docker,
            worktree_path: PathBuf::new(),
            bytes,
            observed_at: now(),
            label: path.rsplit('/').next().unwrap_or(path).to_string(),
            warnings: Vec::new(),
            worktree: None,
        }
    }

    #[test]
    fn the_confirm_names_what_cannot_come_back() {
        let units = vec![
            unit("/w/node_modules", 100, None),
            unit("/w/target", 100, None),
            unit("/w/.cache", 100, None),
            unit(
                "app-data",
                200,
                Some(swamp_core::docker::Removal::Volume {
                    name: "app-data".into(),
                }),
            ),
            unit(
                "sha256:abc",
                300,
                Some(swamp_core::docker::Removal::Image { id: "abc".into() }),
            ),
        ];
        let line = confirm_summary(&units, false);
        // The three the line has room for are all ordinary directories,
        // so without naming them the irreversible units would show only
        // as a byte count.
        assert!(
            line.contains("app-data (docker volume)"),
            "volume not named: {line}"
        );
        assert!(
            line.contains("sha256:abc (docker image)"),
            "image not named: {line}"
        );
        assert!(line.contains("gone for good"), "{line}");
    }

    #[test]
    fn a_plan_with_no_docker_says_nothing_about_permanence() {
        let line = confirm_summary(&[unit("/w/node_modules", 100, None)], false);
        assert!(!line.contains("gone for good"), "{line}");
        assert!(line.contains("→ Trash"), "{line}");
    }

    #[test]
    fn cargo_confirmation_executes_the_reviewed_core_plan() {
        let tmp = tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap().join("repo");
        let selected = root.join("target/debug/incremental/crate-a");
        std::fs::create_dir_all(&selected).unwrap();
        std::fs::write(selected.join("state"), b"state").unwrap();
        std::fs::write(root.join("target/debug/.cargo-lock"), b"").unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='fixture'\nversion='0.1.0'\n",
        )
        .unwrap();
        // A fixture `git init`, counted like every spawn (the
        // reviewer's `every_command_new_must_record_a_spawn`).
        swamp_core::work_counters::record_spawn();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        let store = tempdir().unwrap();
        let report = swamp_core::report::report_full_mode(
            &root,
            None,
            false,
            Some(store.path()),
            Some("1h"),
            true,
            false,
            false,
            true,
        )
        .unwrap();
        let core_plan = swamp_core::actions::propose(
            &report,
            None,
            std::slice::from_ref(&selected),
            "human:tui",
        )
        .unwrap();
        let mut marked = unit(selected.to_str().unwrap(), core_plan.planned_bytes(), None);
        marked.cargo_plan = Some(core_plan);
        let units = vec![marked];
        let (plan, mut grant) = authorize(&units, &swamp_core::authority::HumanConfirmed::tui_dialog("human"));
        let ledger = Ledger::open(store.path().join("ledger.jsonl")).unwrap();
        let trash = store.path().join("Trash");
        grant.created_outside_index = false;
        assert!(
            execute_plan(&units, &plan, &grant, &swamp_core::authority::HumanConfirmed::tui_dialog("human"), &ledger, &trash, false)[0]
                .outcome
                .is_err()
        );
        assert!(selected.exists());
        grant.created_outside_index = true;
        let results = execute_plan(&units, &plan, &grant, &swamp_core::authority::HumanConfirmed::tui_dialog("human"), &ledger, &trash, false);
        assert!(results[0].outcome.is_ok(), "{:?}", results[0].outcome);
        assert!(!selected.exists());
        assert!(
            ledger
                .all()
                .unwrap()
                .iter()
                .any(|r| r.outcome == "completed")
        );
        assert!(
            execute_plan(&units, &plan, &grant, &swamp_core::authority::HumanConfirmed::tui_dialog("human"), &ledger, &trash, false)[0]
                .outcome
                .is_err()
        );
    }

    #[test]
    fn end_to_end_delete_moves_to_trash_and_appends_ledger() {
        let workdir = tempdir().unwrap();
        let target = workdir.path().join("node_modules");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("marker"), b"x").unwrap();
        let bytes = 1024u64;

        let unit = MarkedUnit {
            cargo_plan: None,
            agent_plan: None,
            reviewed: swamp_core::recheck::capture_anchor(&target).ok(),
            path: target.clone(),
            docker: None,
            worktree_path: PathBuf::new(),
            bytes,
            observed_at: now(),
            label: String::new(),
            warnings: Vec::new(),
            worktree: None,
        };
        let (plan, grant) = authorize(std::slice::from_ref(&unit), &swamp_core::authority::HumanConfirmed::tui_dialog("human"));
        let ledger = Ledger::open(workdir.path().join("ledger.jsonl")).unwrap();
        let trash = workdir.path().join("Trash");
        let results = execute_plan(&[unit], &plan, &grant, &swamp_core::authority::HumanConfirmed::tui_dialog("human"), &ledger, &trash, false);

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
            cargo_plan: None,
            agent_plan: None,
            reviewed: None,
            path: "/tmp/target".into(),
            docker: None,
            worktree_path: PathBuf::new(),
            bytes: 2 * 1024 * 1024 * 1024,
            observed_at: 0,
            label: "build target".into(),
            warnings: Vec::new(),
            worktree: None,
        };
        let risky = MarkedUnit {
            cargo_plan: None,
            agent_plan: None,
            reviewed: None,
            path: "/tmp/raw".into(),
            docker: None,
            worktree_path: PathBuf::new(),
            bytes: 1_100_000_000,
            observed_at: 0,
            label: "dir raw".into(),
            warnings: vec!["untracked: in no version control".into()],
            worktree: None,
        };
        let s = confirm_summary(&[clean.clone(), risky.clone()], false);
        assert_eq!(
            s,
            "delete target, raw ⚠ untracked: in no version control (3.2GB) → Trash?  Enter yes · Esc no · k keep executables"
        );
        let s = confirm_summary(&[clean, risky], true);
        assert!(s.ends_with("· keep executables → bin/ (k)"), "{s}");
    }

    #[test]
    fn progress_cancellation_stops_between_units_and_keeps_ledger() {
        let tmp = tempdir().unwrap();
        let units: Vec<_> = (0..3)
            .map(|i| {
                let path = tmp.path().join(format!("cache-{i}"));
                std::fs::create_dir(&path).unwrap();
                std::fs::write(path.join("data"), b"fixture").unwrap();
                unit(path.to_str().unwrap(), 7, None)
            })
            .collect();
        let (plan, grant) = authorize(&units, &swamp_core::authority::HumanConfirmed::tui_dialog("human"));
        let ledger = Ledger::open(tmp.path().join("ledger.jsonl")).unwrap();
        let mut events = Vec::new();
        let results = execute_plan_progress(
            &units,
            &plan,
            &grant,
            &swamp_core::authority::HumanConfirmed::tui_dialog("human"),
            &ledger,
            &tmp.path().join("Trash"),
            false,
            |done, _, outcome| {
                events.push((done, outcome));
                outcome.is_none()
            },
        );
        assert_eq!(results.len(), 1);
        assert!(results[0].outcome.is_ok(), "{:?}", results[0].outcome);
        assert!(!units[0].path.exists());
        assert!(units[1].path.exists() && units[2].path.exists());
        assert_eq!(events, vec![(0, None), (1, Some(true))]);
        assert_eq!(ledger.all().unwrap().len(), 1);
    }
}
