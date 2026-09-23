//! target: crates/core/src/actions.rs
//! expect: retired
//! retired: 2026-09-22 -- the legitimate sink shape written against the pre-gate API (three rechecks by hand, then `std::fs::rename`, which is now a gate path by design); its gate-era form is `accept/02-honoured-sink-moves-with-proof-and-authorization.rs`
//! by: audit:gate_paths_only_inside_gates, compile:E0603
//! why: the legitimate shape must still pass -- all three rechecks, answers honoured, before the rename
pub fn execute_sweep_control(path: &std::path::Path, dir: &std::path::Path) -> anyhow::Result<()> {
    let fresh = crate::recheck::reviewed_snapshot(path, None)?;
    let covered = crate::recheck::covered_paths(&fresh);
    crate::recheck::live_protection(dir, &covered)?;
    match crate::recheck::member_occupancy(&covered) {
        crate::occupancy::OccupancyState::Free => {}
        _ => anyhow::bail!("refused"),
    }
    std::fs::rename(path, dir)?;
    Ok(())
}
