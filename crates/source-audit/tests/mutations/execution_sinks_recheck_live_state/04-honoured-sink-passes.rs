//! target: crates/core/src/actions.rs
//! expect: accept
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
