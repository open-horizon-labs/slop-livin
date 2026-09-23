//! target: crates/core/src/actions.rs
//! mode: append
//! expect: reject
//! source: review-4 sweep (W group, reviewer_counterexamples_stack4_sweep.rs), W5
//! by: audit:gate_paths_only_inside_gates, compile:clippy::disallowed_methods
//! why: W5: a fn item passed as an argument to a generic helper that calls it
//! blind-spot (old model): the argument is a value reference to a std item (dropped); the helper's `f(p)` calls its own parameter
fn sweep4_apply<F: Fn(PathBuf) -> std::io::Result<()>>(f: F, p: &Path) -> std::io::Result<()> {
    f(p.to_path_buf())
}

/// Sweep 4 W5.
pub fn sweep4_w5_prune(path: &Path) -> std::io::Result<()> {
    sweep4_apply(std::fs::remove_dir_all::<PathBuf>, path)
}
