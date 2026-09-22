//! target: crates/core/src/agents/claude_code.rs
//! why: sweep slip -- `use std::fs::read_to_string as slurp` reads a whole transcript past a token match
use std::fs::read_to_string as slurp;
pub fn sweep_slurp(p: &std::path::Path) -> String {
    slurp(p).unwrap_or_default()
}
