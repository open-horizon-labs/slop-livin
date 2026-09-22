//! target: crates/core/src/occupancy.rs
//! why: an alias/rename variant: read_link imported under another name, its error turned into None with .ok(), None read as not held
use std::fs::read_link as peek_link;
pub fn cwd_holds(dir: &Path, target: &Path) -> OccupancyState {
    match peek_link(dir.join("cwd")).ok() {
        Some(p) if p.starts_with(target) => OccupancyState::Occupied(p),
        _ => OccupancyState::Free,
    }
}
