//! target: crates/core/src/growth.rs
//! why: the other half dropped -- deltas appended but the current table never rewritten, so every read replays the whole log
pub mod sweep_delta_only {
    pub fn observe_and_annotate_dirs(dir: &std::path::Path) -> anyhow::Result<()> {
        let _ = super::next_seq_path(dir, "dirs-");
        Ok(())
    }
}
