//! target: crates/core/src/growth.rs
//! mode: append
//! expect: reject
//! source: review-4 sweep (M group, reviewer_counterexamples_stack4_sweep.rs), M21
//! by: audit:gate_paths_only_inside_gates, compile:clippy::disallowed_methods
//! why: `remove_dir_all` of a caller-supplied path with no recheck, written `path.join("")`
//! blind-spot (old model): `caller_supplied` returns false for any subject containing a string literal ("constructed by it"), and `join("")` names exactly the caller's path
/// Sweep 4: the caller's own path, laundered through an empty join.
pub fn sweep4_prune(path: &Path) -> std::io::Result<()> {
    use std::fs as sfs;
    sfs::remove_dir_all(path.join(""))
}
