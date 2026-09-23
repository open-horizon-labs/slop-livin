//! target: crates/core/src/actions.rs
//! mode: append
//! expect: reject
//! source: review-4 sweep (U group, reviewer_counterexamples_stack4_sweep.rs), U3
//! by: audit:gate_paths_only_inside_gates, compile:clippy::disallowed_methods
//! why: U3: the sink written inside a locally defined `macro_rules!`
//! blind-spot (old model): `macro_rules` bodies are skipped, and the invocation's tokens are only `path`
macro_rules! sweep4_zap {
    ($p:expr) => {
        std::fs::remove_dir_all($p)
    };
}

/// Sweep 4 U3.
pub fn sweep4_u3_prune(path: &Path) -> std::io::Result<()> {
    sweep4_zap!(path)
}
