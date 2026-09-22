//! target: crates/core/src/build_adapters/mod.rs
//! why: a container replayed because its own directory stamp did not move -- which it does not when a file inside a subdirectory changes
pub fn sweep_reuse_container(previous_mtime: u64, current_mtime: u64) -> bool {
    previous_mtime == current_mtime
}
