//! target: crates/core/src/actions.rs
//! mode: append
//! expect: reject
//! source: review-4 sweep (W group, reviewer_counterexamples_stack4_sweep.rs), W2
//! by: audit:gate_paths_only_inside_gates, compile:clippy::disallowed_methods
//! why: W2 (owner X5, confirm): a fn pointer stored in a struct field, then called in a sink
//! blind-spot (old model): `(self.op)(path)` is an `ExprCall` whose callee is a field, not a path: no call is recorded
/// Sweep 4 W2.
pub struct Sweep4Pruner {
    pub op: fn(PathBuf) -> std::io::Result<()>,
}

impl Sweep4Pruner {
    pub fn sweep4_new() -> Self {
        Self {
            op: std::fs::remove_dir_all::<PathBuf>,
        }
    }
    pub fn sweep4_prune(&self, path: &Path) -> std::io::Result<()> {
        (self.op)(path.to_path_buf())
    }
}
