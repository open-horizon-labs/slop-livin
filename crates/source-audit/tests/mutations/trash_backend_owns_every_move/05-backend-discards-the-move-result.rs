//! target: crates/core/src/platform/trash.rs
//! why: a discarded-result variant: the move's error is dropped and the item is reported as trashed while it is still at its source
pub fn quiet_move(src: &Path, target: &Target) -> Result<Trashed> {
    let dest = target.probe_path().join("quiet");
    let _ = rename_no_replace(src, &dest);
    Ok(Trashed {
        location: dest,
        info: None,
        kind: Kind::MacOsUser,
        original: src.to_path_buf(),
    })
}
