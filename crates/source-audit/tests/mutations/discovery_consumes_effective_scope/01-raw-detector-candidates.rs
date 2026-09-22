//! target: crates/core/src/external.rs
//! why: the review's excluded_agent_home_must_not_be_scanned counterexample -- discovery reading raw detector output instead of the authorized scope
pub fn sweep_candidates_from_detectors(
    scope: &crate::scope::EffectiveScope,
) -> Vec<std::path::PathBuf> {
    scope
        .detectors
        .iter()
        .filter_map(|d| d.path.clone())
        .collect()
}
