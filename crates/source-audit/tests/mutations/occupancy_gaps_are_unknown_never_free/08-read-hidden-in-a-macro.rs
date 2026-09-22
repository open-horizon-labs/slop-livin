//! target: crates/core/src/occupancy.rs
//! why: a read hidden inside a macro argument, where the call graph cannot follow it
pub fn comm_probe(dir: &Path) -> OccupancyState {
    let name = format!("{}", std::fs::read_to_string(dir.join("comm")).unwrap_or_default());
    if name.is_empty() {
        OccupancyState::Free
    } else {
        OccupancyState::Unknown(name)
    }
}
