//! Which detectors one invocation may run: the only input
//! [`super::Registry::resolve`] accepts besides the environment.
//!
//! `defaults = false` means explicit-only scope
//! (`.oh/guardrails/explicit-only-scope-when-defaults-false.md`): the
//! builtin-defaults detector never runs, and -- unless the config names
//! detectors -- neither does any other one. That decision is made here,
//! from the whole [`crate::scope::ScanConfig`], and nowhere else: the
//! field is private and the only constructor reads the config, so a
//! resolver that is handed the config's parts (`defaults: bool`, an
//! empty disable list) and runs every detector does not compile.

use super::Registry;
use crate::scope::ScanConfig;

/// The detector ids this invocation must not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermittedDetectors {
    disabled: Vec<String>,
}

impl PermittedDetectors {
    pub fn from_config(config: &ScanConfig, registry: &Registry) -> PermittedDetectors {
        let permitted = crate::scope::detectors_permitted(config);
        let mut disabled = config.disabled_detectors.clone();
        if !config.defaults {
            disabled.push(super::builtin::BUILTIN_DEFAULTS_DETECTOR_ID.to_string());
            for d in registry.detectors() {
                let id = d.id().to_string();
                if !permitted
                    || (!config.enabled_detectors.is_empty()
                        && !config.enabled_detectors.contains(&id))
                {
                    disabled.push(id);
                }
            }
        }
        disabled.sort();
        disabled.dedup();
        PermittedDetectors { disabled }
    }

    /// The disabled ids, sorted, as the effective scope records them.
    pub fn disabled(&self) -> Vec<String> {
        self.disabled.clone()
    }

    /// Whether detector `id` may run.
    pub fn permits(&self, id: &str) -> bool {
        !self.disabled.iter().any(|d| d == id)
    }
}
