//! target: crates/core/src/consumer_wiring.rs
//! why: the same second pass hidden inside core, one file away from the observation that owns it
pub fn sweep_core_second_pass(
    scope: &crate::scope::EffectiveScope,
    store: &std::path::Path,
) -> usize {
    crate::external::discover_and_measure(scope, Some(store), false, 1_000, 30, 3600)
        .map(|u| u.len())
        .unwrap_or(0)
}
