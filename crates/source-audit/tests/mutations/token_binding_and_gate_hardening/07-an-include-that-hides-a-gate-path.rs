//! target: crates/core/src/actions.rs
//! mode: append
//! by: audit:gate_paths_only_inside_gates, compile:clippy::disallowed_methods
//! source: re-review 5, finding 6
//! why: a recursive delete spliced into a sink module from a file no `mod` declares: the audit follows `include!` targets into the including module, where the gate path is rejected
include!("sweep5_spliced.rs");
//! file: crates/core/src/sweep5_spliced.rs
//! mode: create
/// Sweep 5: spliced into `actions`.
pub(crate) fn sweep5_spliced_zap(p: &std::path::Path) -> std::io::Result<()> {
    std::fs::remove_dir_all(p)
}
