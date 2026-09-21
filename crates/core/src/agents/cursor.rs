//! Cursor identification (#98): a thin wrapper over
//! `crate::agents::vscode_family::identify_editor_profile`, applied to
//! the `~/Library/Application Support/Cursor` root
//! `crate::locations::cursor::CursorDetector` resolves first. See that
//! detector's doc comment for the community sourcing this is based on,
//! and `vscode_family`'s own doc comment for the shared `state.vscdb`/
//! `workspace.json` identification this tool needs no new concept for.

use super::CandidateAgentUnit;
use std::path::Path;

pub const CURSOR_TOOL_ID: &str = crate::locations::cursor::CURSOR_DETECTOR_ID;

pub fn identify(profile_root: &Path, observed_at: u64) -> Vec<CandidateAgentUnit> {
    super::vscode_family::identify_editor_profile(profile_root, observed_at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{AgentActionCapability, AgentCategory};
    use std::fs;

    fn touch(path: &Path, content: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn identifies_the_global_database_as_protected() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        touch(&root.join("User/globalStorage/state.vscdb"), b"sqlite");
        let units = identify(root, 1);
        let u = units
            .iter()
            .find(|u| u.category == AgentCategory::Sessions)
            .expect("global db identified");
        assert!(u.protected);
        assert_eq!(u.action, AgentActionCapability::None);
    }
}
