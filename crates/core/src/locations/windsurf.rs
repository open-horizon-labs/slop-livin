//! Windsurf editor-profile storage: `~/Library/Application Support/
//! Windsurf` (macOS), plus the separate `~/.codeium/windsurf` root.
//!
//! Lower confidence than every other row in this catalog, disclosed
//! honestly rather than hidden: Windsurf's own documentation
//! (docs.windsurf.com) redirected to `docs.devin.ai/desktop/...` during
//! this chunk's research (checked during implementation, never learned
//! from a real Windsurf installation on this machine -- PRIVACY IS A
//! HARD RULE) -- consistent with Windsurf's acquisition/product
//! consolidation, and meaning no current primary documentation of its
//! on-disk layout was reachable this chunk. What is modeled here:
//! - `~/.codeium/windsurf`: named in the issue text and in
//!   <https://registry.coder.com/modules/coder/windsurf> (a Coder
//!   community module) as holding MCP config and agent config; treated
//!   as a second, non-decomposed location, same as Cursor's `~/.cursor`.
//! - `~/Library/Application Support/Windsurf`: Windsurf is a VS Code
//!   fork, so this chunk *assumes* the same `User/{globalStorage,
//!   workspaceStorage,History}` shape Cursor's own fork uses
//!   (`crate::locations::cursor`), reusing
//!   `crate::agents::vscode_family`'s shared identification for exactly
//!   that reason -- but this assumption is **not independently
//!   confirmed** for Windsurf specifically this chunk, unlike Cursor's
//!   community-sourced-but-corroborated shape. Anything this adapter
//!   gets wrong about Windsurf's actual database layout would show up as
//!   an honest "(unsupported layout version)" residual rather than a
//!   silent miscount, via the same version-marker check
//!   `crate::agents::vscode_family` gives every VS-Code-family adapter.
//!
//! Platform-gated to macOS only this chunk, same reasoning as Cursor's.

use super::{
    Detector, Environment, LocationStatus, Platform, ProposedLocation, Provenance, StorageCategory,
};

pub const WINDSURF_DETECTOR_ID: &str = "windsurf";

pub struct WindsurfDetector;

impl Detector for WindsurfDetector {
    fn id(&self) -> &'static str {
        WINDSURF_DETECTOR_ID
    }

    fn name(&self) -> &'static str {
        "Windsurf"
    }

    fn platforms(&self) -> &'static [Platform] {
        &[Platform::MacOS]
    }

    fn version_note(&self) -> &'static str {
        "docs.windsurf.com redirects to docs.devin.ai as of this chunk; no current primary \
         documentation of the on-disk layout was reachable. The Application Support/Windsurf \
         shape is assumed (VS Code fork), not confirmed, unlike Cursor's row -- see this module's \
         doc comment and crate::agents::vscode_family's version-marker fallback"
    }

    fn detect(&self, env: &Environment) -> Vec<ProposedLocation> {
        let app_support = env
            .home
            .join("Library")
            .join("Application Support")
            .join("Windsurf");
        let codeium_root = env.home.join(".codeium").join("windsurf");
        vec![
            ProposedLocation {
                detector_id: WINDSURF_DETECTOR_ID.to_string(),
                path: Some(app_support),
                category: StorageCategory::LocalState,
                provenance: Provenance::BuiltinConvention,
                status: LocationStatus::Resolved,
                note: Some(
                    "Windsurf editor-profile root, assumed VS-Code-fork shape (not \
                     independently confirmed this chunk); see crate::agents::windsurf"
                        .to_string(),
                ),
            },
            ProposedLocation {
                detector_id: WINDSURF_DETECTOR_ID.to_string(),
                path: Some(codeium_root),
                category: StorageCategory::LocalState,
                provenance: Provenance::BuiltinConvention,
                status: LocationStatus::Resolved,
                note: Some(
                    "Windsurf MCP/agent config root (~/.codeium/windsurf); reported as an \
                     opaque external unit, not decomposed"
                        .to_string(),
                ),
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    #[test]
    fn convention_paths() {
        let env =
            Environment::fixture(PathBuf::from("/Users/dev"), HashMap::new(), Platform::MacOS);
        let got = WindsurfDetector.detect(&env);
        assert_eq!(got.len(), 2);
        assert_eq!(
            got[0].path,
            Some(PathBuf::from(
                "/Users/dev/Library/Application Support/Windsurf"
            ))
        );
        assert_eq!(
            got[1].path,
            Some(PathBuf::from("/Users/dev/.codeium/windsurf"))
        );
    }

    #[test]
    fn macos_only() {
        assert_eq!(WindsurfDetector.platforms(), &[Platform::MacOS]);
    }
}
