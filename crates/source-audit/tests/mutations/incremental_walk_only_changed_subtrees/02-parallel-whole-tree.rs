//! target: crates/core/src/growth.rs
//! why: the same regression written through the whole-tree attribution pass instead of full_walk
pub mod sweep_incremental_parallel {
    pub fn apply_incremental(
        root: &std::path::Path,
        _changed: &[std::path::PathBuf],
        observed_at: u64,
    ) -> anyhow::Result<()> {
        let _ = crate::walk::attribute_parallel(root, &[], observed_at, 0);
        Ok(())
    }
}
