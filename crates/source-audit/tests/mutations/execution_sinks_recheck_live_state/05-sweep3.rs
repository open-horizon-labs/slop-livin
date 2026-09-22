//! target: crates/core/src/growth.rs
//! mode: append
//! why: re-review 3 sweep -- `fs::remove_dir_all` on a caller-supplied path with no recheck at all, written in growth.rs (blind spot: the audit skips recheck.rs, store.rs and growth.rs *by whole file*, so any destructive primitive placed in one of them is unaudited)
/// Sweep: a destructive sink in a file the audit skips wholesale.
pub fn sweep_prune_unit(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::remove_dir_all(path)
}
