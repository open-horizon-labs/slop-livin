//! Codex desktop app identification (#93): logs only. See
//! `crate::locations::codex_desktop` for why this adapter deliberately
//! covers less than `crate::agents::codex` does -- the desktop app's
//! settings/session storage is not confirmed by primary source this
//! chunk, and this adapter says so rather than guessing.

use super::{AgentActionCapability, AgentCategory, CandidateAgentUnit, folded_bytes};
use std::fs;
use std::path::Path;

pub const CODEX_DESKTOP_TOOL_ID: &str = crate::locations::codex_desktop::CODEX_DESKTOP_DETECTOR_ID;

const MAX_FOLD_ENTRIES: usize = 200_000;

/// `home` here is the *log directory itself*
/// (`~/Library/Logs/com.openai.codex`), not a tool home with an interior
/// to decompose further -- there is nothing else confirmed under it.
/// The whole directory is one folded, actionable Logs-category unit,
/// same recovery contract as any other tool's log directory.
pub fn identify(home: &Path, _observed_at: u64) -> Vec<CandidateAgentUnit> {
    if !home.exists() {
        return Vec::new();
    }
    // A freshly created, still-empty log directory is not yet worth a
    // row (see the identical note in `oh_my_pi::unknown_format_residual`
    // on why an entry check, not `folded_bytes`'s mtime, decides this).
    let has_entries = fs::read_dir(home)
        .map(|mut rd| rd.next().is_some())
        .unwrap_or(false);
    if !has_entries {
        return Vec::new();
    }
    let (bytes, mtime, truncated) = folded_bytes(home, MAX_FOLD_ENTRIES);
    let note = if truncated {
        "desktop app log directory (date-tree session logs); directory entry count bound \
         reached, total may be an undercount"
            .to_string()
    } else {
        "desktop app log directory (date-tree session logs); settings/session storage beyond \
         logs is an unconfirmed layout, not modeled here"
            .to_string()
    };
    vec![CandidateAgentUnit {
        category: AgentCategory::Logs,
        relative_path: "(log directory)".to_string(),
        path: home.to_path_buf(),
        members: Vec::new(),
        bytes,
        mtime_max: mtime,
        protected: false,
        protect_reason: None,
        project_link: super::ProjectLinkState::NotApplicable,
        action: AgentActionCapability::CacheOrLogTrash,
        note: Some(note),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn missing_directory_yields_no_units() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert!(identify(&missing, 1).is_empty());
    }

    #[test]
    fn log_files_are_folded_into_one_actionable_unit() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("2026/09/21");
        fs::create_dir_all(&day).unwrap();
        fs::write(day.join("session.log"), b"line 1\nline 2\n").unwrap();
        let units = identify(dir.path(), 1);
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].category, AgentCategory::Logs);
        assert_eq!(units[0].action, AgentActionCapability::CacheOrLogTrash);
        assert!(!units[0].protected);
        assert!(units[0].bytes > 0);
    }
}
