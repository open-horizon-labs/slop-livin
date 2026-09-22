//! target: crates/core/src/actions.rs
//! why: sweep slip -- `let _ = recheck::live_protection(..)` keeps the call and throws the answer away
pub fn execute_sweep_mutation_a(dir: &std::path::Path, path: &std::path::Path) -> anyhow::Result<()> {
    let _ = crate::recheck::reviewed_snapshot(path, None);
    let _ = crate::recheck::live_protection(dir, &[]);
    let _ = crate::recheck::member_occupancy(&[]);
    std::fs::rename(path, dir)?;
    Ok(())
}
