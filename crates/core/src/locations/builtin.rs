//! The three built-in default scan roots, as a detector (#41/#44).
//!
//! Modeled as a detector -- not a hard-coded list inside `scope.rs` --
//! so `[scan] defaults = false` and `disabled_detectors` are the same
//! mechanism a real detector uses: `crate::scope::resolve_effective_scope`
//! folds `defaults = false` into "also disable `builtin-defaults`" before
//! calling [`super::Registry::resolve`]. See `crate::scope` for exactly
//! how that folding happens; this file only proposes the candidates.

use super::{
    Detector, Environment, LocationStatus, Platform, ProposedLocation, Provenance, StorageCategory,
};

pub const BUILTIN_DEFAULTS_DETECTOR_ID: &str = "builtin-defaults";

pub struct BuiltinDefaultsDetector;

impl Detector for BuiltinDefaultsDetector {
    fn id(&self) -> &'static str {
        BUILTIN_DEFAULTS_DETECTOR_ID
    }

    fn name(&self) -> &'static str {
        "Built-in default roots"
    }

    fn platforms(&self) -> &'static [Platform] {
        // Linux's own default table is #84's job. Leaving this detector's
        // Linux arm empty (rather than reusing the macOS paths) is what
        // keeps a macOS-specific `~/Library/...` path from ever appearing
        // in a Linux-configured `Environment`, fixture or real.
        &[Platform::MacOS, Platform::Linux]
    }

    fn version_note(&self) -> &'static str {
        "macOS: ~/src, ~/Library/Developer, ~/Library/Caches; Linux: none yet (#84)"
    }

    fn detect(&self, env: &Environment) -> Vec<ProposedLocation> {
        let candidates: &[(&str, &str)] = match env.platform {
            Platform::MacOS => &[
                (
                    "src",
                    "checkouts a developer keeps under a home-relative src tree",
                ),
                (
                    "Library/Developer",
                    "Xcode/Android build, SDK, and simulator storage",
                ),
                ("Library/Caches", "the platform-wide user cache directory"),
            ],
            Platform::Linux => &[],
        };
        candidates
            .iter()
            .map(|(rel, note)| ProposedLocation {
                detector_id: BUILTIN_DEFAULTS_DETECTOR_ID.to_string(),
                path: Some(env.home.join(rel)),
                category: StorageCategory::Unclassified,
                provenance: Provenance::BuiltinConvention,
                status: LocationStatus::Resolved,
                note: Some(note.to_string()),
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
    fn macos_proposes_three_roots() {
        let env =
            Environment::fixture(PathBuf::from("/Users/dev"), HashMap::new(), Platform::MacOS);
        let got = BuiltinDefaultsDetector.detect(&env);
        let paths: Vec<_> = got.iter().filter_map(|l| l.path.clone()).collect();
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/Users/dev/src"),
                PathBuf::from("/Users/dev/Library/Developer"),
                PathBuf::from("/Users/dev/Library/Caches"),
            ]
        );
        assert!(got.iter().all(|l| l.status == LocationStatus::Resolved));
    }

    #[test]
    fn linux_proposes_nothing_yet_never_leaks_macos_paths() {
        let env = Environment::fixture(PathBuf::from("/home/dev"), HashMap::new(), Platform::Linux);
        let got = BuiltinDefaultsDetector.detect(&env);
        assert!(
            got.is_empty(),
            "Linux defaults are #84's job, not a silent copy of macOS's"
        );
    }
}
