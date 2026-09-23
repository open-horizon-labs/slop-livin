//! target: crates/core/src/actions.rs
//! mode: append
//! expect: reject
//! by: compile:E0603, audit:gate_paths_only_inside_gates
//! ported: 2026-09-22 -- actions.rs no longer imports `std::fs` as `fs`; the removal spelled out
//! source: review-4 sweep (M group, reviewer_counterexamples_stack4_sweep.rs), M27
//! why: a removal that proceeds when `matches!(state, Free | Unknown(_))`
//! blind-spot (old model): a `matches!` collapses the tri-state only when its pattern names `Occupied` and not `Unknown`; naming `Unknown` on the *proceed* side passes
/// Sweep 4: an unanswerable probe reads as quiet.
pub fn sweep4_remove_if_quiet(p: &Path) -> Result<()> {
    let members = vec![p.to_path_buf()];
    let state = crate::recheck::member_occupancy(&members);
    if matches!(
        state,
        crate::occupancy::OccupancyState::Free | crate::occupancy::OccupancyState::Unknown(_)
    ) {
        std::fs::remove_file(p)?;
    }
    Ok(())
}
