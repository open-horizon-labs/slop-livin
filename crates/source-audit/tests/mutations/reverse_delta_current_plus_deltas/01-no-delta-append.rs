//! target: crates/core/src/growth.rs
//! why: history rewritten in place with no reverse delta -- past observations become unreconstructible
pub mod sweep_no_delta {
    pub fn observe_and_annotate(dir: &std::path::Path) -> anyhow::Result<()> {
        let _ = super::current_path(dir);
        Ok(())
    }
}
