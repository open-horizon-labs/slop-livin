//! target: crates/core/src/build_adapters/jvm_common.rs
//! why: the neutral helper reaching the action layer through a function no name list contains -- the whole `actions` namespace is the hazard
pub fn sweep_listed(store: &std::path::Path) -> usize {
    crate::actions::list_plans(store).map(|p| p.len()).unwrap_or(0)
}
