//! target: crates/core/src/occupancy.rs
//! why: a let-else on a failed read that answers Free
pub fn exe_holds(dir: &Path, target: &Path) -> OccupancyState {
    let Ok(exe) = std::fs::read_link(dir.join("exe")) else {
        return OccupancyState::Free;
    };
    if exe.starts_with(target) {
        OccupancyState::Occupied(exe)
    } else {
        OccupancyState::Free
    }
}
