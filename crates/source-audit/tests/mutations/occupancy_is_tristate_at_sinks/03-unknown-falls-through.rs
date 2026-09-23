//! target: crates/core/src/actions.rs
//! by: audit:gate_paths_only_inside_gates, compile:E0603
//! why: an Unknown arm that does not refuse is a fail-open occupancy gate
pub fn sweep_unknown_falls_through(paths: &[std::path::PathBuf]) -> bool {
    match crate::recheck::member_occupancy(paths) {
        crate::occupancy::OccupancyState::Free => true,
        crate::occupancy::OccupancyState::Occupied(_) => false,
        crate::occupancy::OccupancyState::Unknown(_) => true,
    }
}
