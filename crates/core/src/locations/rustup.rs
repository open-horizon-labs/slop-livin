//! rustup home: `RUSTUP_HOME` override, else `~/.rustup`.
//! https://rust-lang.github.io/rustup/environment-variables.html

use super::{
    Detector, Environment, LocationStatus, Platform, ProposedLocation, Provenance, StorageCategory,
};

pub const RUSTUP_DETECTOR_ID: &str = "rustup";

pub struct RustupDetector;

impl Detector for RustupDetector {
    fn id(&self) -> &'static str {
        RUSTUP_DETECTOR_ID
    }

    fn name(&self) -> &'static str {
        "rustup"
    }

    fn platforms(&self) -> &'static [Platform] {
        &[Platform::MacOS, Platform::Linux]
    }

    fn version_note(&self) -> &'static str {
        "rustup environment-variables reference, current stable"
    }

    fn detect(&self, env: &Environment) -> Vec<ProposedLocation> {
        let (base, provenance) = match env.env_var("RUSTUP_HOME") {
            Some(v) if !v.is_empty() => (
                std::path::PathBuf::from(v),
                Provenance::EnvVar("RUSTUP_HOME".to_string()),
            ),
            _ => (env.home.join(".rustup"), Provenance::BuiltinConvention),
        };
        vec![
            ProposedLocation {
                detector_id: RUSTUP_DETECTOR_ID.to_string(),
                path: Some(base.clone()),
                category: StorageCategory::Installation,
                provenance: provenance.clone(),
                status: LocationStatus::Resolved,
                note: Some("installed toolchains and components".to_string()),
            },
            ProposedLocation {
                detector_id: RUSTUP_DETECTOR_ID.to_string(),
                path: Some(base.join("downloads")),
                category: StorageCategory::Downloads,
                provenance,
                status: LocationStatus::Resolved,
                note: Some("in-progress/partial toolchain download staging".to_string()),
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
        let got = RustupDetector.detect(&env);
        assert_eq!(got[0].path, Some(PathBuf::from("/Users/dev/.rustup")));
        assert!(matches!(got[0].provenance, Provenance::BuiltinConvention));
    }

    #[test]
    fn env_var_override_wins() {
        let mut env_vars = HashMap::new();
        env_vars.insert("RUSTUP_HOME".to_string(), "/opt/rustup-home".to_string());
        let env = Environment::fixture(PathBuf::from("/Users/dev"), env_vars, Platform::MacOS);
        let got = RustupDetector.detect(&env);
        assert_eq!(got[0].path, Some(PathBuf::from("/opt/rustup-home")));
        assert_eq!(
            got[0].provenance,
            Provenance::EnvVar("RUSTUP_HOME".to_string())
        );
    }
}
