//! target: crates/core/src/occupancy.rs
//! why: a discarded-result variant: the probe reads the process table and throws the answer away, then reports Free
pub fn warm_probe(proc_root: &Path) -> OccupancyState {
    let _ = std::fs::read_dir(proc_root);
    OccupancyState::Free
}
