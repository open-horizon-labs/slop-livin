//! target: crates/core/src/agents/registry.rs
//! why: an adapter registered twice, so one tool's units are identified and counted twice
pub fn sweep_extra_registration() -> Vec<Box<dyn crate::agents::AgentAdapter>> {
    vec![Box::new(codex::Adapter)]
}
