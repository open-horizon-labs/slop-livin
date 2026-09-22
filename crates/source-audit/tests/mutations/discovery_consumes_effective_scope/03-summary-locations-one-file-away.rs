//! target: crates/core/src/agents/codex.rs
//! why: the same bypass moved one adapter away, reading the scope summary's raw locations
pub fn sweep_homes_from_summary(summary: &crate::scope::ScopeSummary) -> Vec<std::path::PathBuf> {
    summary
        .locations
        .iter()
        .filter_map(|l| l.path.clone())
        .collect()
}
