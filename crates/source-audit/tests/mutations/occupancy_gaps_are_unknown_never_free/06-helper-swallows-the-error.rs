//! target: crates/core/src/occupancy.rs
//! why: the probe delegates to a helper one call away that answers "not visible" as false with is_ok
pub fn visible_probe(dir: &Path) -> OccupancyState {
    if fd_table_listed(dir) {
        OccupancyState::Free
    } else {
        OccupancyState::Occupied(dir.to_path_buf())
    }
}
fn fd_table_listed(dir: &Path) -> bool {
    std::fs::read_dir(dir.join("fd")).is_ok()
}
