//! target: crates/core/src/recheck.rs
//! mode: append
//! expect: accept
//! source: review-4 sweep (A group, reviewer_counterexamples_stack4_sweep.rs), A1
//! ported: 2026-09-22 -- under the gate an occupancy answer is consumed only in `recheck` (the sinks take it through `run_all`), so the reviewer's shape now lives there, and the function is private (a public item nothing names is dead public API)
//! why: A1: `Unknown` refused first, then `Occupied` refused -- fail closed, two `if let`s
//! blind-spot (old model): the `if let` scan flags any pattern naming `Occupied` without `Unknown`, without asking whether `Unknown` was already refused
/// Sweep 4 A1: refuses both non-free answers.
fn sweep4_a1_guard(p: &Path) -> Result<()> {
    let members = vec![p.to_path_buf()];
    let state = member_occupancy(&members);
    if let OccupancyState::Unknown(why) = &state {
        bail!("occupancy unknown: {why}");
    }
    if let OccupancyState::Occupied(_) = &state {
        bail!("occupied");
    }
    Ok(())
}
