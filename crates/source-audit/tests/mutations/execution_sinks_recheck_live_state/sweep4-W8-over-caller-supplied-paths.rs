//! target: crates/core/src/actions.rs
//! mode: append
//! expect: reject
//! source: review-4 sweep (W group, reviewer_counterexamples_stack4_sweep.rs), W8
//! by: audit:gate_paths_only_inside_gates, compile:clippy::disallowed_methods
//! why: W8: `try_for_each(std::fs::remove_dir_all)` over caller-supplied paths
//! blind-spot (old model): the most idiomatic spelling of the class: a fn item handed to an iterator adaptor is an argument, not a call
/// Sweep 4 W8.
pub fn sweep4_w8_prune_all(paths: &[PathBuf]) -> std::io::Result<()> {
    paths.iter().try_for_each(std::fs::remove_dir_all)
}
