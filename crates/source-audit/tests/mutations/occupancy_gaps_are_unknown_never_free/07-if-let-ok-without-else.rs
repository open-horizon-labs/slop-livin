//! target: crates/core/src/occupancy.rs
//! why: if let Ok with no else: an unreadable cwd link is skipped silently and the loop ends in Free
pub fn cwd_scan(dirs: &[std::path::PathBuf], target: &Path) -> OccupancyState {
    for d in dirs {
        if let Ok(cwd) = std::fs::read_link(d.join("cwd")) {
            if cwd.starts_with(target) {
                return OccupancyState::Occupied(cwd);
            }
        }
    }
    OccupancyState::Free
}
