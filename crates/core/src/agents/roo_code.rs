//! Roo Code identification (#99): a thin wrapper over
//! `crate::agents::vscode_family::identify_extension_globalstorage`,
//! applied to each host location
//! `crate::locations::roo_code::RooCodeDetector` resolves. Same
//! per-host, never-merged discipline as `crate::agents::cline`.
//!
//! One thing worth restating from `crate::locations::roo_code`'s doc
//! comment: a Roo Code task directory has been community-reported to
//! embed a full Git checkpoint repository, so `vscode_family`'s folded
//! byte total for a task is not necessarily small -- this adapter does
//! not special-case that; `SessionRemoval`'s existing loss warning
//! already covers "removes this session's resume/rewind/checkpoint
//! history" generically (`crate::actions::unit_from_agent`).

use super::CandidateAgentUnit;
use std::path::Path;

pub const ROO_CODE_TOOL_ID: &str = crate::locations::roo_code::ROO_CODE_DETECTOR_ID;

pub fn identify(ext_home: &Path, observed_at: u64) -> Vec<CandidateAgentUnit> {
    super::vscode_family::identify_extension_globalstorage(ext_home, observed_at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn a_task_under_the_remote_host_is_labelled() {
        let dir = tempfile::tempdir().unwrap();
        let ext_home = dir
            .path()
            .join(".vscode-server/data/User/globalStorage/rooveterinaryinc.roo-cline");
        fs::create_dir_all(ext_home.join("tasks/t1")).unwrap();
        fs::write(ext_home.join("tasks/t1/ui_messages.json"), b"[]").unwrap();
        let units = identify(&ext_home, 1);
        assert_eq!(units.len(), 1);
        assert!(
            units[0]
                .relative_path
                .starts_with("VS Code Server (remote)/tasks/")
        );
    }
}
