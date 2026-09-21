//! Cline (`saoudrizwan.claude-dev`) VS Code extension storage: one
//! `globalStorage/saoudrizwan.claude-dev/` location per known editor
//! host (`crate::locations::vscode_hosts`), never a single merged path
//! -- the same extension ID installed in VS Code, VS Code Insiders,
//! Cursor and Windsurf each has its own, genuinely separate, on-disk
//! storage.
//!
//! No single official layout-reference document was found this chunk
//! (checked during implementation, never learned from a real Cline
//! installation on this machine -- PRIVACY IS A HARD RULE);
//! community-documented via GitHub issue threads:
//! - <https://github.com/cline/cline/issues/7101>: confirms data lives
//!   under the extension's `globalStorage` directory and that corrupted
//!   JSON task-history files have historically been silently deleted --
//!   named directly in #99's own guardrail text as a reason this
//!   adapter never touches a task file's *contents*, only its metadata
//!   shape.
//! - <https://github.com/cline/cline/issues/14135>: further corroborates
//!   the `tasks/<task-id>/` per-task directory shape.
//! - This adapter's own per-task file names
//!   (`api_conversation_history.json`, `ui_messages.json`,
//!   `task_metadata.json`) and the `workspace` field it reads from
//!   `task_metadata.json` for project linkage are the issue text's own
//!   research, not independently re-confirmed against a schema doc this
//!   chunk -- see `crate::agents::vscode_family`'s doc comment for how a
//!   missing/renamed field degrades to `Unresolved`, never a guess.

use super::vscode_hosts::globalstorage_candidates;
use super::{Detector, Environment, LocationStatus, Platform, ProposedLocation, StorageCategory};

pub const CLINE_DETECTOR_ID: &str = "cline";
pub const CLINE_EXTENSION_ID: &str = "saoudrizwan.claude-dev";

pub struct ClineDetector;

impl Detector for ClineDetector {
    fn id(&self) -> &'static str {
        CLINE_DETECTOR_ID
    }

    fn name(&self) -> &'static str {
        "Cline"
    }

    fn platforms(&self) -> &'static [Platform] {
        &[Platform::MacOS]
    }

    fn version_note(&self) -> &'static str {
        "community-documented (github.com/cline/cline issues #7101, #14135), no official \
         layout-reference doc found this chunk; macOS hosts only, Linux deferred to the Linux \
         track (#77-#89); see crate::agents::vscode_family"
    }

    fn detect(&self, env: &Environment) -> Vec<ProposedLocation> {
        globalstorage_candidates(&env.home, CLINE_EXTENSION_ID)
            .into_iter()
            .map(|(host, path)| ProposedLocation {
                detector_id: CLINE_DETECTOR_ID.to_string(),
                path: Some(path),
                category: StorageCategory::LocalState,
                provenance: super::Provenance::BuiltinConvention,
                status: LocationStatus::Resolved,
                note: Some(format!(
                    "Cline globalStorage under the {host} host: tasks/<task-id>/ (conversation \
                     history, UI messages, checkpoints); see crate::agents::vscode_family"
                )),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    #[test]
    fn one_location_per_known_host() {
        let env =
            Environment::fixture(PathBuf::from("/Users/dev"), HashMap::new(), Platform::MacOS);
        let got = ClineDetector.detect(&env);
        assert_eq!(got.len(), 5);
        assert!(got.iter().any(|l| {
            l.path
                == Some(PathBuf::from(
                    "/Users/dev/Library/Application Support/Code/User/globalStorage/\
                     saoudrizwan.claude-dev",
                ))
        }));
        assert!(got.iter().any(|l| {
            l.path
                == Some(PathBuf::from(
                    "/Users/dev/Library/Application Support/Cursor/User/globalStorage/\
                     saoudrizwan.claude-dev",
                ))
        }));
    }
}
