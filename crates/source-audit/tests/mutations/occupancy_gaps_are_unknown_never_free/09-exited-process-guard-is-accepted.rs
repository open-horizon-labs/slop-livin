//! target: crates/core/src/occupancy.rs
//! expect: accept
//! why: skipping a process that exited mid-scan (NotFound) is correct: it holds nothing, and every other error is Unknown
pub fn exited_ok(dirs: &[std::path::PathBuf], target: &Path) -> OccupancyState {
    for d in dirs {
        match std::fs::read_link(d.join("cwd")) {
            Ok(cwd) if cwd.starts_with(target) => return OccupancyState::Occupied(cwd),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return OccupancyState::Unknown(e.to_string()),
        }
    }
    OccupancyState::Free
}
