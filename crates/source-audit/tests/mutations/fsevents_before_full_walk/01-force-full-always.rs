//! target: crates/core/src/growth.rs
//! why: sweep slip -- the entry point runs a full walk itself instead of delegating replay planning
pub fn observe_tracked_with_source(root: &std::path::Path) -> u64 {
    let _ = crate::walk::full_walk(root);
    0
}
