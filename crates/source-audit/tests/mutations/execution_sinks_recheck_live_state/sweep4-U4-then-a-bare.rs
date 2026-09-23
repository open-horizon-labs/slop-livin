//! target: crates/core/src/actions.rs
//! mode: append
//! expect: reject
//! source: review-4 sweep (U group, reviewer_counterexamples_stack4_sweep.rs), U4
//! by: audit:gate_paths_only_inside_gates, compile:clippy::disallowed_methods
//! why: U4: `use std::fs::*;` then a bare `remove_dir_all(path)`
//! blind-spot (old model): glob imports are recorded (`Fun::globs`) and consulted only for *local* free functions; a bare std name resolves to itself and `fs::remove_dir_all` does not suffix-match `remove_dir_all`
/// Sweep 4 U4.
pub mod sweep4_u4 {
    use std::fs::*;
    use std::path::Path;
    pub fn sweep4_u4_prune(path: &Path) -> std::io::Result<()> {
        remove_dir_all(path)
    }
}
