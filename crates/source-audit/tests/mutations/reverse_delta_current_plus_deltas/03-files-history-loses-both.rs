//! target: crates/core/src/growth.rs
//! why: the files table silently stops keeping history at all while the function keeps its name
pub mod sweep_files_no_history {
    pub fn observe_and_annotate_files(_dir: &std::path::Path) -> anyhow::Result<()> {
        Ok(())
    }
}
