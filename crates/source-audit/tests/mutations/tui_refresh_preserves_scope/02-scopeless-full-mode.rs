//! target: crates/tui/src/model.rs
//! why: the same refresh moved one file away, through a different scopeless entry point
pub fn sweep_full_refresh(root: &std::path::Path) -> anyhow::Result<swamp_core::report::Report> {
    swamp_core::report::report_full_mode(root, 1_000, true)
}
