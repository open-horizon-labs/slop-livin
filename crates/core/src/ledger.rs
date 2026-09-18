use crate::grants::Verb;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};
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
            fs::create_dir_all(p)?;
        }
        Ok(Self { path })
    }
    pub fn append(&self, r: &ActionRecord) -> Result<()> {
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{}", serde_json::to_string(r)?)?;
        f.sync_data()?;
        Ok(())
    }
    pub fn all(&self) -> Result<Vec<ActionRecord>> {
        if !self.path.exists() {
            return Ok(vec![]);
        }
        let f = fs::File::open(&self.path)?;
        BufReader::new(f)
            .lines()
            .map(|l| Ok(serde_json::from_str(&l?)?))
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
