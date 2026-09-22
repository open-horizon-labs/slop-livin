//! target: crates/core/src/actions.rs
//! why: the sink keeps its rechecks and hands the unit to a helper one call away that renames it into the trash
pub fn execute_via_helper(unit: &Path, trash: &Path) -> Result<()> {
    if !crate::recheck::member_occupancy(&[unit.to_path_buf()]).is_free() {
        bail!("in use");
    }
    stash_into_trash(unit, trash)
}
fn stash_into_trash(unit: &Path, trash: &Path) -> Result<()> {
    std::fs::rename(unit, trash.join("stashed"))?;
    Ok(())
}
