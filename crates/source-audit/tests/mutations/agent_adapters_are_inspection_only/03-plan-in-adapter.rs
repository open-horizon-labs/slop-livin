//! target: crates/core/src/agents/opencode.rs
//! why: an adapter that reaches the action layer is not inspection-only
pub fn sweep_adapter_plans(units: &[crate::agents::AgentUnit], paths: &[std::path::PathBuf]) {
    let _ = crate::actions::propose_agents(units, paths, "adapter");
}
