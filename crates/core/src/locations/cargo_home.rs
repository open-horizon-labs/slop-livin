//! Cargo home: `CARGO_HOME` override, else `~/.cargo` (Cargo Book,
//! "cargo-home"). https://doc.rust-lang.org/cargo/guide/cargo-home.html
//!
//! Reports the home directory itself (installation-adjacent: `bin/`,
//! `config.toml`, credentials) separately from the two large, safely
//! re-downloadable subtrees so a consumer can treat them differently --
//! this registry only proposes locations, it does not judge disposal.

use super::{
    Detector, Environment, LocationStatus, Platform, ProposedLocation, Provenance, StorageCategory,
};

pub const CARGO_HOME_DETECTOR_ID: &str = "cargo-home";

pub struct CargoHomeDetector;

impl Detector for CargoHomeDetector {
    fn id(&self) -> &'static str {
        CARGO_HOME_DETECTOR_ID
    }

    fn name(&self) -> &'static str {
        "Cargo home"
    }

    fn platforms(&self) -> &'static [Platform] {
        &[Platform::MacOS, Platform::Linux]
    }

    fn version_note(&self) -> &'static str {
        "Cargo Book cargo-home layout, current stable"
    }

    fn detect(&self, env: &Environment) -> Vec<ProposedLocation> {
        let (base, provenance) = match env.env_var("CARGO_HOME") {
            Some(v) if !v.is_empty() => (
                std::path::PathBuf::from(v),
                Provenance::EnvVar("CARGO_HOME".to_string()),
            ),
            _ => (env.home.join(".cargo"), Provenance::BuiltinConvention),
        };
        vec![
            ProposedLocation {
                detector_id: CARGO_HOME_DETECTOR_ID.to_string(),
                path: Some(base.clone()),
                category: StorageCategory::Installation,
                provenance: provenance.clone(),
                status: LocationStatus::Resolved,
                note: Some("cargo home: bin/, config.toml, credentials".to_string()),
            },
            ProposedLocation {
                detector_id: CARGO_HOME_DETECTOR_ID.to_string(),
                path: Some(base.join("registry")),
                category: StorageCategory::Cache,
                provenance: provenance.clone(),
                status: LocationStatus::Resolved,
                note: Some("downloaded crate sources and registry index cache".to_string()),
            },
            ProposedLocation {
                detector_id: CARGO_HOME_DETECTOR_ID.to_string(),
                path: Some(base.join("git")),
                category: StorageCategory::Cache,
                provenance,
                status: LocationStatus::Resolved,
                note: Some("checked-out git dependencies".to_string()),
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
    fn convention_when_no_env_override() {
        let env =
            Environment::fixture(PathBuf::from("/Users/dev"), HashMap::new(), Platform::MacOS);
        let got = CargoHomeDetector.detect(&env);
        assert_eq!(got[0].path, Some(PathBuf::from("/Users/dev/.cargo")));
        assert_eq!(
            got[1].path,
            Some(PathBuf::from("/Users/dev/.cargo/registry"))
        );
        assert_eq!(got[2].path, Some(PathBuf::from("/Users/dev/.cargo/git")));
        assert!(matches!(got[0].provenance, Provenance::BuiltinConvention));
    }

    #[test]
    fn env_var_override_wins_and_is_labelled() {
        let mut env_vars = HashMap::new();
        env_vars.insert("CARGO_HOME".to_string(), "/opt/cargo-home".to_string());
        let env = Environment::fixture(PathBuf::from("/Users/dev"), env_vars, Platform::MacOS);
        let got = CargoHomeDetector.detect(&env);
        assert_eq!(got[0].path, Some(PathBuf::from("/opt/cargo-home")));
        assert_eq!(
            got[0].provenance,
            Provenance::EnvVar("CARGO_HOME".to_string())
        );
    }

    #[test]
    fn leftovers_found_without_executable_present() {
        // The detector never checks whether `cargo` is on PATH; a
        // convention probe still proposes the path. Existence is scope
        // resolution's job (`crate::scope`), not the detector's.
        let env =
            Environment::fixture(PathBuf::from("/Users/dev"), HashMap::new(), Platform::MacOS);
        let got = CargoHomeDetector.detect(&env);
        assert!(got.iter().all(|l| l.status == LocationStatus::Resolved));
    }
}
