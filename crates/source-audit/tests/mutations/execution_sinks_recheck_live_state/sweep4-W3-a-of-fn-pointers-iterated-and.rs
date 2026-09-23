//! target: crates/core/src/actions.rs
//! mode: append
//! expect: reject
//! source: review-4 sweep (W group, reviewer_counterexamples_stack4_sweep.rs), W3
//! by: audit:gate_paths_only_inside_gates, compile:clippy::disallowed_methods
//! why: W3: a `Vec` of fn pointers iterated and applied to a caller-supplied path
//! blind-spot (old model): the fn item is a value inside `vec![..]`; `op(path)` names a loop binding
/// Sweep 4 W3.
pub fn sweep4_w3_prune(path: &Path) -> std::io::Result<()> {
    let ops: Vec<fn(PathBuf) -> std::io::Result<()>> = vec![std::fs::remove_dir_all::<PathBuf>];
    for op in &ops {
        op(path.to_path_buf())?;
    }
    Ok(())
}
