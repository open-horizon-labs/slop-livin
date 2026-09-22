//! target: crates/core/src/report.rs
//! why: a pipeline stage called directly instead of through the bus, so no consumer can observe or replace it
pub fn sweep_direct_stage(root: &std::path::Path) {
    let _ = crate::walk::discover_and_attribute(root, 1_000, 0);
}
