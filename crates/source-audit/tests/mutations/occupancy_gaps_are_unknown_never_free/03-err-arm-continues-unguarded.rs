//! target: crates/core/src/occupancy.rs
//! why: an fd table that cannot be listed (permission denied, not a process that exited) is skipped with continue
pub fn fd_scan(dirs: &[std::path::PathBuf], target: &Path) -> OccupancyState {
    for d in dirs {
        let fds = match std::fs::read_dir(d.join("fd")) {
            Ok(fds) => fds,
            Err(_) => continue,
        };
        for fd in fds {
            let Ok(fd) = fd else { return OccupancyState::Unknown("fd".into()) };
            if fd.path().starts_with(target) {
                return OccupancyState::Occupied(fd.path());
            }
        }
    }
    OccupancyState::Free
}
