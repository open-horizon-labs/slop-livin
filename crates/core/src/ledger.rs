use crate::grants::Verb;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRecord {
    pub id: String,
    pub verb: Verb,
    pub entity_id: String,
    pub evidence: serde_json::Value,
    pub grant_id: String,
    pub actor: String,
    pub outcome: String,
    pub recovery_location: Option<PathBuf>,
    pub measured_free_space_delta: Option<i64>,
    pub observed_path_state: Option<String>,
    pub recorded_at: u64,
}
#[derive(Debug, Clone)]
pub struct Ledger {
    path: PathBuf,
}
impl Ledger {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(p) = path.parent() {
            crate::fs_gate::store::create_dir_all(p)?;
        }
        Ok(Self { path })
    }
    pub fn append(&self, r: &ActionRecord) -> Result<()> {
        crate::fs_gate::store::append_line(
            crate::fs_gate::store::LogFile::Ledger(&self.path),
            &serde_json::to_string(r)?,
        )?;
        Ok(())
    }
    pub fn all(&self) -> Result<Vec<ActionRecord>> {
        let lines = match crate::fs_gate::read::read_owned_lines(&self.path) {
            Ok(lines) => lines,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e.into()),
        };
        lines
            .iter()
            .map(|l| Ok(serde_json::from_str(l)?))
            .collect()
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::now;
    use tempfile::tempdir;
    #[test]
    fn ledger_survives_index_removal() {
        let d = tempdir().unwrap();
        let l = Ledger::open(d.path().join("ledger.jsonl")).unwrap();
        let r = ActionRecord {
            id: "1".into(),
            verb: Verb::Delete,
            entity_id: "e".into(),
            evidence: serde_json::json!({"bytes":3}),
            grant_id: "g".into(),
            actor: "human".into(),
            outcome: "completed".into(),
            recovery_location: None,
            measured_free_space_delta: Some(2),
            observed_path_state: Some("gone".into()),
            recorded_at: now(),
        };
        l.append(&r).unwrap();
        assert_eq!(l.all().unwrap()[0].entity_id, "e");
    }
}
