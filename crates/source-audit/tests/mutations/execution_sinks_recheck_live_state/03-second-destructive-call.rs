//! target: crates/core/src/actions.rs
//! why: sweep slip -- only the FIRST destructive call's prefix was inspected, so a second one after the rechecks was free
pub fn execute_sweep_mutation_c(
    path: &std::path::Path,
    dir: &std::path::Path,
    other: &std::path::Path,
) -> anyhow::Result<()> {
    let fresh = crate::recheck::reviewed_snapshot(path, None)?;
    let covered = crate::recheck::covered_paths(&fresh);
    crate::recheck::live_protection(dir, &covered)?;
    match crate::recheck::member_occupancy(&covered) {
        crate::occupancy::OccupancyState::Free => {}
        _ => anyhow::bail!("refused"),
    }
    std::fs::rename(path, dir)?;
    // A second, unrechecked path.
    std::fs::remove_dir_all(other)?;
    Ok(())
}
