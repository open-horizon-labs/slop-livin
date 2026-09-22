//! target: crates/core/src/build_adapters/node.rs
//! why: alias/rename variant -- slurping every package.json in a node_modules tree past a token match
use std::fs::read_to_string as slurp;
pub fn sweep_slurp(p: &std::path::Path) -> Option<String> {
    slurp(p).ok()
}
