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
    /// Remove a linked git worktree (directory to Trash, then
    /// `git worktree prune`). Its own bar: never Main/Clone checkouts, and
    /// only when the worktree is clean, has nothing unpushed, is unlocked
    /// and unoccupied at the sink.
    RemoveWorktree,
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn index_cannot_mint_authorization() {
        let artifact = Artifact {
            id: "a".into(),
            project_id: None,
            kind: ArtifactKind::BuildOutput,
            path: "/tmp/a".into(),
            relative_path: None,
            bytes: 1,
            recovery: RecoveryContract::LocalRebuild,
            present: true,
            regrowth_count: 0,
            meta: FactMeta::now("test", Confidence::High),
        };
        let grant = Grant {
            id: "g".into(),
            verb: Verb::Delete,
            predicate: Predicate {
                project_id: None,
                kind: None,
                max_bytes: 10,
                require_fresh_within_secs: 60,
            },
            scope: vec![],
            expires_at: now() + 60,
            actor: "agent".into(),
            created_outside_index: false,
        };
        assert!(grant.validate(&artifact, now()).is_err());
    }
}
