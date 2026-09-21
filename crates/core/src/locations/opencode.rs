//! OpenCode (sst/opencode) storage locations. Unlike Claude Code/Codex/Oh
//! My Pi, OpenCode does not keep everything under one directory: data,
//! config and cache are three independent roots, confirmed by primary
//! source during implementation (never a real `~/.local/share/opencode`
//! on this machine -- PRIVACY IS A HARD RULE):
//! - <https://opencode.ai/docs/troubleshooting/>: data
//!   `~/.local/share/opencode/` (macOS/Linux) holding `auth.json`, `log/`
//!   and per-project session storage; cache `~/.cache/opencode/`
//!   (provider package downloads); config `~/.config/opencode/`
//!   (`opencode.jsonc`/`.json`, local plugin directories).
//! - <https://github.com/anomalyco/opencode/issues/6669> and
//!   <https://github.com/anomalyco/opencode/issues/18633>: config
//!   follows `XDG_CONFIG_HOME`; data/state currently lands under
//!   `XDG_DATA_HOME` even where upstream's own issue tracker argues some
//!   of it belongs under `XDG_STATE_HOME` instead -- this detector
//!   follows the *current* (as of this chunk) behavior, not the
//!   aspirational one, and says so.
//! - `OPENCODE_DATA_DIR` as a direct data-root override is carried over
//!   from this epic's own prior research (`crate::agents::matrix`'s
//!   Planned-row note); this chunk did not find an independent primary
//!   source confirming the literal env var name, so it is honored here
//!   defensively (an unset env var is a no-op) but flagged as
//!   unconfirmed rather than presented as verified.
//!
//! This detector proposes the **data root first**: `crate::agents`'s
//! shared discovery orchestration (`discover_and_measure`) uses a
//! detector's *first* `Resolved` location as the tool's "home" for
//! interior identification, so the data root -- where sessions,
//! snapshots and the newer SQLite store actually live -- must be first.
//! Config and cache are reported too (so they are visible as ordinary
//! external units with their own byte totals), but are not decomposed
//! into `AgentUnit`s: config is small, protected-by-nature (holds
//! `opencode.jsonc`), and cache is an opaque, wholly re-downloadable
//! provider-package cache -- neither carries session/project linkage,
//! and `crate::agents::opencode`'s interior identification is reserved
//! for the data root where that linkage actually lives.

use super::{
    Detector, Environment, LocationStatus, Platform, ProposedLocation, Provenance, StorageCategory,
};
use std::path::PathBuf;

pub const OPENCODE_DETECTOR_ID: &str = "opencode";

pub struct OpenCodeDetector;

fn xdg_or_home(env: &Environment, xdg_var: &str, home_suffix: &[&str]) -> (PathBuf, Provenance) {
    match env.env_var(xdg_var) {
        Some(v) if !v.is_empty() => (
            PathBuf::from(v).join("opencode"),
            Provenance::EnvVar(xdg_var.to_string()),
        ),
        _ => {
            let mut base = env.home.clone();
            for part in home_suffix {
                base = base.join(part);
            }
            (base.join("opencode"), Provenance::BuiltinConvention)
        }
    }
}

impl Detector for OpenCodeDetector {
    fn id(&self) -> &'static str {
        OPENCODE_DETECTOR_ID
    }

    fn name(&self) -> &'static str {
        "OpenCode"
    }

    fn platforms(&self) -> &'static [Platform] {
        &[Platform::MacOS, Platform::Linux]
    }

    fn version_note(&self) -> &'static str {
        "opencode.ai/docs/troubleshooting and github.com/anomalyco/opencode issues #6669/#18633, \
         current as of this chunk; version-specific storage.ts layout vs. opencode.db is \
         resolved by crate::agents::opencode, not by this detector"
    }

    fn detect(&self, env: &Environment) -> Vec<ProposedLocation> {
        let (data, data_provenance) = match env.env_var("OPENCODE_DATA_DIR") {
            Some(v) if !v.is_empty() => (
                PathBuf::from(v),
                Provenance::EnvVar("OPENCODE_DATA_DIR".to_string()),
            ),
            _ => xdg_or_home(env, "XDG_DATA_HOME", &[".local", "share"]),
        };
        let (config, config_provenance) = xdg_or_home(env, "XDG_CONFIG_HOME", &[".config"]);
        let (cache, cache_provenance) = xdg_or_home(env, "XDG_CACHE_HOME", &[".cache"]);
        vec![
            ProposedLocation {
                detector_id: OPENCODE_DETECTOR_ID.to_string(),
                path: Some(data),
                category: StorageCategory::LocalState,
                provenance: data_provenance,
                status: LocationStatus::Resolved,
                note: Some(
                    "OpenCode data root: auth, logs, session/message/project storage (file tree \
                     or opencode.db depending on version) and git-backed snapshots; see \
                     crate::agents::opencode"
                        .to_string(),
                ),
            },
            ProposedLocation {
                detector_id: OPENCODE_DETECTOR_ID.to_string(),
                path: Some(config),
                category: StorageCategory::LocalState,
                provenance: config_provenance,
                status: LocationStatus::Resolved,
                note: Some(
                    "OpenCode config root (opencode.jsonc, plugins); reported as an opaque \
                     external unit, not decomposed into agent units"
                        .to_string(),
                ),
            },
            ProposedLocation {
                detector_id: OPENCODE_DETECTOR_ID.to_string(),
                path: Some(cache),
                category: StorageCategory::Cache,
                provenance: cache_provenance,
                status: LocationStatus::Resolved,
                note: Some(
                    "OpenCode provider-package cache; wholly re-downloadable, reported as an \
                     opaque external unit"
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

    #[test]
    fn convention_paths_with_no_overrides() {
        let env =
            Environment::fixture(PathBuf::from("/Users/dev"), HashMap::new(), Platform::MacOS);
        let got = OpenCodeDetector.detect(&env);
        assert_eq!(got.len(), 3);
        assert_eq!(
            got[0].path,
            Some(PathBuf::from("/Users/dev/.local/share/opencode"))
        );
        assert_eq!(
            got[1].path,
            Some(PathBuf::from("/Users/dev/.config/opencode"))
        );
        assert_eq!(
            got[2].path,
            Some(PathBuf::from("/Users/dev/.cache/opencode"))
        );
    }

    #[test]
    fn data_dir_env_override_wins_over_xdg() {
        let mut env_vars = HashMap::new();
        env_vars.insert("OPENCODE_DATA_DIR".to_string(), "/opt/oc-data".to_string());
        env_vars.insert("XDG_DATA_HOME".to_string(), "/opt/xdg-data".to_string());
        let env = Environment::fixture(PathBuf::from("/Users/dev"), env_vars, Platform::MacOS);
        let got = OpenCodeDetector.detect(&env);
        assert_eq!(got[0].path, Some(PathBuf::from("/opt/oc-data")));
    }

    #[test]
    fn xdg_overrides_are_each_honored_independently() {
        let mut env_vars = HashMap::new();
        env_vars.insert("XDG_DATA_HOME".to_string(), "/opt/xdg-data".to_string());
        env_vars.insert("XDG_CONFIG_HOME".to_string(), "/opt/xdg-config".to_string());
        env_vars.insert("XDG_CACHE_HOME".to_string(), "/opt/xdg-cache".to_string());
        let env = Environment::fixture(PathBuf::from("/Users/dev"), env_vars, Platform::MacOS);
        let got = OpenCodeDetector.detect(&env);
        assert_eq!(got[0].path, Some(PathBuf::from("/opt/xdg-data/opencode")));
        assert_eq!(got[1].path, Some(PathBuf::from("/opt/xdg-config/opencode")));
        assert_eq!(got[2].path, Some(PathBuf::from("/opt/xdg-cache/opencode")));
    }

    #[test]
    fn data_root_is_first_so_agent_identification_uses_it() {
        let env = Environment::fixture(PathBuf::from("/home/dev"), HashMap::new(), Platform::Linux);
        let got = OpenCodeDetector.detect(&env);
        assert_eq!(
            got[0].path,
            Some(PathBuf::from("/home/dev/.local/share/opencode"))
        );
        assert_eq!(got[0].category, StorageCategory::LocalState);
    }
}
