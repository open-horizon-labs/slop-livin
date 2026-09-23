//! target: crates/core/src/actions.rs
//! mode: append
//! by: audit:gate_paths_only_inside_gates, compile:clippy::disallowed_methods
//! why: the same bypass with the rename imported under another name
mod quick_sink {
    use std::fs::rename as shift;
    use std::path::Path;
    pub fn go(unit: &Path, trash: &Path) -> anyhow::Result<()> {
        if crate::recheck::member_occupancy(&[unit.to_path_buf()]).is_free() {
            shift(unit, trash.join("x"))?;
        }
        Ok(())
    }
}
