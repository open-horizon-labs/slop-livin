//! target: crates/core/src/actions.rs
//! mode: append
//! expect: reject
//! source: review-4 sweep (W group, reviewer_counterexamples_stack4_sweep.rs), W9
//! by: audit:gate_paths_only_inside_gates, compile:clippy::disallowed_methods
//! why: W9: a `const` fn pointer to `remove_dir_all`, called in a sink
//! blind-spot (old model): item-level consts are recorded (`Program::items`) and never become edges; calling the const names the const
const SWEEP4_ZAP: fn(PathBuf) -> std::io::Result<()> = std::fs::remove_dir_all::<PathBuf>;

/// Sweep 4 W9.
pub fn sweep4_w9_prune(path: &Path) -> std::io::Result<()> {
    SWEEP4_ZAP(path.to_path_buf())
}
