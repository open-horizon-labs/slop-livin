//! Windsurf identification (#98): a thin wrapper over
//! `crate::agents::vscode_family::identify_editor_profile`, applied to
//! the `~/Library/Application Support/Windsurf` root
//! `crate::locations::windsurf::WindsurfDetector` resolves first. See
//! that detector's doc comment for why this shape is assumed (VS Code
//! fork) rather than independently confirmed for Windsurf specifically
//! -- an installation whose actual layout differs shows up as
//! `vscode_family`'s own honest "(unsupported layout version)" residual,
//! never a silent miscount.

use super::CandidateAgentUnit;
use std::path::Path;

pub const WINDSURF_TOOL_ID: &str = crate::locations::windsurf::WINDSURF_DETECTOR_ID;

pub fn identify(profile_root: &Path, observed_at: u64) -> Vec<CandidateAgentUnit> {
    super::vscode_family::identify_editor_profile(profile_root, observed_at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn unrecognized_layout_is_an_explicit_residual() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        fs::create_dir_all(root).unwrap();
        fs::write(root.join("unrelated.txt"), b"hello").unwrap();
        let units = identify(root, 1);
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].relative_path, "(unsupported layout version)");
    }
}
