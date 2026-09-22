//! target: crates/tui/src/app.rs
//! why: the agent half of the same second pass, run from the TUI instead of taken from the observation
pub fn sweep_tui_agents(
    scope: &swamp_core::scope::EffectiveScope,
    store: &std::path::Path,
) -> usize {
    swamp_core::agents::discover_and_measure(scope, &[], Some(store), true, 1_000, 30, 3600)
        .map(|u| u.len())
        .unwrap_or(0)
}
