use crate::fs_gate::store::{LogFile, StoreDir};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// What one ledger line records happened.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Verb {
    Delete,
    Archive,
    /// Remove a linked git worktree (directory to Trash, then
    /// `git worktree prune`).
    RemoveWorktree,
}

/// Written into every [`ActionRecord::grant_id`]: there is no grant any
/// more, and this says so plainly rather than fabricating an id.
pub const NO_GRANT: &str = "human-marked";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRecord {
    pub id: String,
    pub verb: Verb,
    pub entity_id: String,
    pub evidence: serde_json::Value,
    /// Historical field name; there is no grant any more. Kept so a
    /// ledger written by an older swamp still deserializes, and set to
    /// the constant below by every writer now.
    pub grant_id: String,
    pub actor: String,
    pub outcome: String,
    pub recovery_location: Option<PathBuf>,
    pub measured_free_space_delta: Option<i64>,
    pub observed_path_state: Option<String>,
    pub recorded_at: u64,
}
/// The append-only action ledger. Its location is a [`LogFile`]: a
/// store's `ledger.jsonl`, or the `$SWAMP_LEDGER_PATH` override read in
/// the gate -- never an arbitrary file a caller names.
#[derive(Debug, Clone)]
pub struct Ledger {
    store: StoreDir,
    resolved: bool,
}
impl Ledger {
    /// `<dir>/ledger.jsonl`. `path` must name a file called
    /// `ledger.jsonl`: the ledger appends to nothing else.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if path.file_name().is_none_or(|n| n != "ledger.jsonl") {
            anyhow::bail!(
                "{} is not a swamp ledger (a ledger is a store's `ledger.jsonl`)",
                path.display()
            );
        }
        let dir = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("{} has no store directory", path.display()))?;
        Ok(Self {
            store: StoreDir::at(dir)?,
            resolved: false,
        })
    }
    /// The ledger for `store`, honouring `$SWAMP_LEDGER_PATH` (the TUI).
    pub fn resolved(store: &StoreDir) -> Self {
        Self {
            store: store.clone(),
            resolved: true,
        }
    }
    fn file(&self) -> LogFile<'_> {
        if self.resolved {
            LogFile::LedgerResolved(&self.store)
        } else {
            LogFile::Ledger(&self.store)
        }
    }
    pub fn append(&self, r: &ActionRecord) -> Result<()> {
        crate::fs_gate::store::append_line(self.file(), &serde_json::to_string(r)?)?;
        Ok(())
    }
    pub fn all(&self) -> Result<Vec<ActionRecord>> {
        let lines = match crate::fs_gate::read::read_owned_lines(self.path()) {
            Ok(lines) => lines,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e.into()),
        };
        lines.iter().map(|l| Ok(serde_json::from_str(l)?)).collect()
    }
    pub fn path(&self) -> PathBuf {
        self.file()
            .path()
            .unwrap_or_else(|_| self.store.path().join("ledger.jsonl"))
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
