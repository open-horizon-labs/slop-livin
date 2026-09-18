use crate::{entities::*, grants::*, ledger::*};
use anyhow::Result;
use std::{fs, path::Path};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Outcome {
    pub unit_id: String,
    pub status: String,
    pub reason: Option<String>,
    pub intended_bytes: u64,
    pub observed_free_space_delta: Option<i64>,
}

/// Execute only after the caller supplies a human-originated grant. The
/// filesystem is re-read immediately before the sink; an index row is never
/// accepted as a substitute for that observation.
pub fn execute_delete(
    artifact: &Artifact,
    plan_unit: &PlanUnit,
    grant: &Grant,
    ledger: &Ledger,
    trash_root: &Path,
    actor: &str,
) -> Result<Outcome> {
    if plan_unit.artifact_id != artifact.id {
        anyhow::bail!("plan unit does not identify artifact")
    }
    grant.validate(artifact, now())?;
    if crate::occupancy::occupied(&artifact.path) {
        anyhow::bail!("occupied")
    }
    let current = fs::symlink_metadata(&artifact.path)
        .map_err(|_| anyhow::anyhow!("could not re-observe path"))?;
    if current.len() != artifact.bytes && current.is_file() {
        anyhow::bail!("activity changed")
    }
    let destination = trash_root.join(format!("{}-{}", now(), artifact.id));
    fs::create_dir_all(trash_root)?;
    fs::rename(&artifact.path, &destination)?;
    let outcome = Outcome {
        unit_id: artifact.id.clone(),
        status: "completed".into(),
        reason: None,
        intended_bytes: artifact.bytes,
        observed_free_space_delta: None,
    };
    ledger.append(&ActionRecord{id:new_id(),verb:grant.verb.clone(),entity_id:artifact.id.clone(),evidence:serde_json::json!({"observed_at":artifact.meta.observed_at,"bytes":artifact.bytes}),grant_id:grant.id.clone(),actor:actor.into(),outcome:"completed".into(),recovery_location:Some(destination),measured_free_space_delta:None,observed_path_state:Some("trashed".into()),recorded_at:now()})?;
    Ok(outcome)
}
