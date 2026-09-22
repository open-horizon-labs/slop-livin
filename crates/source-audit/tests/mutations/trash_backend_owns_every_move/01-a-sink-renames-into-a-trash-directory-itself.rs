//! target: crates/core/src/actions.rs
//! why: a new sink rechecks occupancy and then renames the unit into a trash directory itself, bypassing the backend's no-copy/no-fallback rules and its restore record
pub fn execute_quick(unit: &Path, trash: &Path) -> Result<PathBuf> {
    match crate::recheck::member_occupancy(&[unit.to_path_buf()]) {
        crate::occupancy::OccupancyState::Free => {}
        other => bail!("{:?}", other),
    }
    let dest = trash.join("quick");
    fs::rename(unit, &dest)?;
    Ok(dest)
}
