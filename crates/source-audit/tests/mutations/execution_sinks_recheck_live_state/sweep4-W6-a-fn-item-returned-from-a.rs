//! target: crates/core/src/actions.rs
//! mode: append
//! expect: reject
//! source: review-4 sweep (W group, reviewer_counterexamples_stack4_sweep.rs), W6
//! by: audit:gate_paths_only_inside_gates, compile:clippy::disallowed_methods
//! why: W6: a fn item returned from a helper and called on the spot
//! blind-spot (old model): `zap()(path)` has a non-path callee; `zap` references a std item only as a value
fn sweep4_zap() -> fn(PathBuf) -> std::io::Result<()> {
    std::fs::remove_dir_all::<PathBuf>
}

/// Sweep 4 W6.
pub fn sweep4_w6_prune(path: &Path) -> std::io::Result<()> {
    sweep4_zap()(path.to_path_buf())
}
