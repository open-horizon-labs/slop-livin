//! target: crates/tui/src/app.rs
//! why: the store-interior spelling of the external pass run from the TUI -- a second observation of the same stores and their history family
pub fn sweep_second_external_pass(
    scope: &swamp_core::scope::EffectiveScope,
    store: &std::path::Path,
) -> usize {
    swamp_core::external::observe_external(
        scope,
        Some(store),
        true,
        0,
        30,
        3600,
        &swamp_core::fs_events::EventCoverage::untrusted(),
    )
    .map(|o| o.interiors.len())
    .unwrap_or(0)
}
