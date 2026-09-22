//! target: crates/core/src/growth.rs
//! why: the targeted re-walk removed altogether -- changed directories are recorded and never re-measured
pub mod sweep_incremental_noop {
    pub fn apply_incremental(
        _root: &std::path::Path,
        changed: &[std::path::PathBuf],
        _observed_at: u64,
    ) -> anyhow::Result<usize> {
        Ok(changed.len())
    }
}
