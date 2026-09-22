//! target: crates/core/src/growth.rs
//! why: a staged replay that walks fully before consulting FSEvents is the original non-incremental defect
pub fn stage_tracked_with_source(root: &std::path::Path, src: &dyn crate::fs_events::FsEventsSource) -> u64 {
    let _ = crate::walk::full_walk(root);
    let _ = src.replay(root);
    0
}
