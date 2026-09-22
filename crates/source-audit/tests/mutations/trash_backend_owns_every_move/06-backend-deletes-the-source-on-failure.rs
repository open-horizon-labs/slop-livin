//! target: crates/core/src/platform/trash.rs
//! why: a permanent-deletion fallback -- when the rename fails, the backend removes the source instead
pub fn move_or_drop(src: &Path, dst: &Path) -> Result<()> {
    if rename_no_replace(src, dst).is_err() {
        std::fs::remove_file(src)?;
    }
    Ok(())
}
