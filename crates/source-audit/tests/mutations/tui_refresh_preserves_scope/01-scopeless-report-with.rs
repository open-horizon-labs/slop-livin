//! target: crates/tui/src/app.rs
//! why: CE3's shape -- a refresh through a scopeless entry point, so excluded subtrees reappear
pub fn sweep_refresh(root: &std::path::Path) -> anyhow::Result<swamp_core::report::Report> {
    swamp_core::report::report_with(root, 1_000)
}
