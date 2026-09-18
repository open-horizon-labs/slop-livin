use crate::entities::*;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Verb {
    Leave,
    Prevent,
    Delete,
    Archive,
    BypassTrash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Predicate {
    pub project_id: Option<String>,
    pub kind: Option<ArtifactKind>,
    pub max_bytes: u64,
    pub require_fresh_within_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Grant {
    pub id: String,
    pub verb: Verb,
    pub predicate: Predicate,
    pub scope: Vec<String>,
    pub expires_at: u64,
    pub actor: String,
    pub created_outside_index: bool,
}

impl Grant {
    pub fn validate(&self, artifact: &Artifact, at: u64) -> Result<()> {
        if !self.created_outside_index {
            anyhow::bail!("grant provenance is not human-authorized")
        }
        if at > self.expires_at {
            anyhow::bail!("grant expired")
        }
        if self.predicate.max_bytes > 0 && artifact.bytes > self.predicate.max_bytes {
            anyhow::bail!("blast-radius budget exceeded")
        }
        if let Some(p) = &self.predicate.project_id
            && artifact.project_id.as_ref() != Some(p)
        {
            anyhow::bail!("project outside grant scope")
        }
        if let Some(k) = &self.predicate.kind
            && &artifact.kind != k
        {
            anyhow::bail!("kind outside grant scope")
        }
        if now().saturating_sub(artifact.meta.observed_at)
            > self.predicate.require_fresh_within_secs
        {
            anyhow::bail!("fact is stale")
        }
        if self.verb == Verb::Delete && artifact.recovery == RecoveryContract::Irrecoverable {
            anyhow::bail!("irrecoverable artifact requires leave")
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanUnit {
    pub artifact_id: String,
    pub verb: Verb,
    pub expected_bytes: u64,
    pub evidence_observed_at: u64,
    pub undo_cost: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub units: Vec<PlanUnit>,
    pub expires_at: u64,
    pub single_use: bool,
}
pub fn plan(units: Vec<PlanUnit>, expires_at: u64) -> Plan {
    Plan {
        id: new_id(),
        units,
        expires_at,
        single_use: true,
    }
}
