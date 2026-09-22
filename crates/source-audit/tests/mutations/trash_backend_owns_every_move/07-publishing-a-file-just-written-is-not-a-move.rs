//! target: crates/core/src/actions.rs
//! expect: accept
//! why: write-then-rename of a control file this function just wrote, reachable from a sink, is an atomic publish and not a move of user data
pub fn record_quick(unit: &Path, store: &Path) -> Result<()> {
    if !crate::recheck::member_occupancy(&[unit.to_path_buf()]).is_free() {
        bail!("in use");
    }
    write_quick_marker(store)
}
fn write_quick_marker(store: &Path) -> Result<()> {
    let tmp = store.join("marker.tmp");
    fs::write(&tmp, b"x")?;
    fs::rename(&tmp, store.join("marker"))?;
    Ok(())
}
