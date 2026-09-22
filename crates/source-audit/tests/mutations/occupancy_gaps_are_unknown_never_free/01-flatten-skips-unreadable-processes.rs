//! target: crates/core/src/occupancy.rs
//! why: flattening the process listing drops every entry that could not be read, so an invisible process reads as one holding nothing
pub fn procfs_quick(proc_root: &Path, target: &Path) -> OccupancyState {
    for entry in std::fs::read_dir(proc_root).into_iter().flatten().flatten() {
        if entry.path().join("cwd").starts_with(target) {
            return OccupancyState::Occupied(entry.path());
        }
    }
    OccupancyState::Free
}
