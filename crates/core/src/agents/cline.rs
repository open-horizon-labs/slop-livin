//! Cline identification (#99): a thin wrapper over
//! `crate::agents::vscode_family::identify_extension_globalstorage`,
//! applied to each host location
//! `crate::locations::cline::ClineDetector` resolves (#99's explicit
//! "model each host as a separate detector location" -- `identify` is
//! called once per host by `crate::agents::discover_and_measure`'s
//! multi-location iteration, never merged).

use super::CandidateAgentUnit;
use std::path::Path;

pub const CLINE_TOOL_ID: &str = crate::locations::cline::CLINE_DETECTOR_ID;

pub fn identify(ext_home: &Path, observed_at: u64) -> Vec<CandidateAgentUnit> {
    super::vscode_family::identify_extension_globalstorage(ext_home, observed_at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::AgentActionCapability;
    use std::fs;

    #[test]
    fn a_task_is_identified_with_its_host_label() {
        let dir = tempfile::tempdir().unwrap();
        let ext_home = dir
            .path()
            .join("Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev");
        fs::create_dir_all(ext_home.join("tasks/t1")).unwrap();
        fs::write(
            ext_home.join("tasks/t1/api_conversation_history.json"),
            b"[]",
        )
        .unwrap();
        let units = identify(&ext_home, 1);
        assert_eq!(units.len(), 1);
        assert!(units[0].relative_path.starts_with("VS Code/tasks/"));
        assert_eq!(units[0].action, AgentActionCapability::SessionRemoval);
    }
}
