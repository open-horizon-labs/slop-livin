//! target: crates/core/src/platform/trash.rs
//! why: the backend grows the trash crate's EXDEV fallback -- copy the tree, then delete the source -- which duplicates bytes, splits hardlinks and is not atomic
pub fn move_across(src: &Path, dst: &Path) -> Result<()> {
    if rename_no_replace(src, dst).is_err() {
        std::fs::copy(src, dst)?;
        std::fs::remove_dir_all(src)?;
    }
    Ok(())
}
