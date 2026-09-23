//! target: crates/core/src/actions.rs
//! mode: append
//! expect: reject
//! source: review-4 sweep (U group, reviewer_counterexamples_stack4_sweep.rs), U2
//! by: audit:gate_paths_only_inside_gates
//! why: U2: all three rechecks present, honoured -- inside `#[cfg(any())]`, which the compiler removes
//! blind-spot (old model): the model parses text, not the compiled crate: code under a false `cfg` satisfies every *required-call* rule (the owner tried the dual, an extra sink under `cfg(not(test))`, which is stricter and was caught)
/// Sweep 4 U2.
pub fn sweep4_u2_prune(store_dir: &Path, path: &Path) -> Result<()> {
    let paths = vec![path.to_path_buf()];
    #[cfg(any())]
    {
        crate::recheck::reviewed_snapshot(path, None)?;
        crate::recheck::live_protection(store_dir, &paths)?;
        if !matches!(
            crate::recheck::member_occupancy(&paths),
            crate::occupancy::OccupancyState::Free
        ) {
            bail!("occupied");
        }
    }
    let _ = (store_dir, &paths);
    fs::remove_dir_all(path)?;
    Ok(())
}
