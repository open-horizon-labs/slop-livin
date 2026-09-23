//! target: crates/core/src/signals.rs
//! mode: append
//! by: audit:gate_paths_only_inside_gates, compile:E0433
//! source: re-review 5, finding 5
//! why: a `gix_*` crate (here `gix_fs`, whose `symlink::remove` is `std::fs::remove_file`) named directly, outside the gate
/// Sweep 5: gitoxide's filesystem layer, reached around `gix`.
pub(crate) fn sweep5_unlink(p: &std::path::Path) {
    let _ = gix_fs::symlink::remove(p);
}
