//! target: crates/core/src/actions.rs
//! why: sweep slip -- the tri-state answer is obtained and thrown away
pub fn sweep_discard_occupancy(paths: &[std::path::PathBuf]) {
    let _ = crate::recheck::member_occupancy(paths);
}
