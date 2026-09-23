//! target: crates/core/src/ignore.rs
//! mode: append
//! by: audit:gate_paths_only_inside_gates, compile:clippy::disallowed_methods
//! source: re-review 5, finding 5
//! why: gitoxide opened outside `fs_gate::git` -- `gix` re-exports file removal, spawning, temp and lock files, so an ungated repository handle is an ungated filesystem and process capability
/// Sweep 5: a repository opened beside the gate.
pub(crate) fn sweep5_repo_is_open(root: &std::path::Path) -> bool {
    gix::open(root).is_ok()
}
