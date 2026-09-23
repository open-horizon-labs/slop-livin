//! target: crates/core/src/actions.rs
//! by: audit:gate_paths_only_inside_gates, compile:clippy::disallowed_methods
//! why: sweep slip -- `use std::fs::rename as move_aside` hides the destructive primitive from a token match
use std::fs::rename as move_aside;
pub fn execute_sweep_mutation_b(from: &std::path::Path, to: &std::path::Path) -> anyhow::Result<()> {
    move_aside(from, to)?;
    Ok(())
}
