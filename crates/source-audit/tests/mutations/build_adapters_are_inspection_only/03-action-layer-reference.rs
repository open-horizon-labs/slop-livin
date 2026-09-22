//! target: crates/core/src/build_adapters/maven.rs
//! why: identification reaching the plan/grant layer is the "inspection is not authorization" break
pub fn sweep_plan(p: &std::path::Path) -> Option<crate::actions::Plan> {
    crate::actions::propose_for_path(p)
}
