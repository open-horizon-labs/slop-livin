//! target: crates/core/src/recheck.rs
//! expect: accept
//! source: accept
//! why: an exhaustive `match` on the tri-state in the recheck, proceeding only on `Free`
/// Accept: only `Free` proceeds.
fn sweep_accept_free(members: &[PathBuf]) -> Result<()> {
    match member_occupancy(members) {
        OccupancyState::Free => Ok(()),
        OccupancyState::Occupied(p) => bail!("{} is open", p.display()),
        OccupancyState::Unknown(why) => bail!("occupancy unknown: {why}"),
    }
}
