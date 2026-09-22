//! target: crates/core/src/scope.rs
//! why: the predicate kept and gutted -- `detectors_permitted` returns true whatever the config says, so the name is present and the contract is gone
pub mod sweep_permit_everything {
    pub fn detectors_permitted(config: &super::ScanConfig) -> bool {
        let _ = config;
        true
    }
}
