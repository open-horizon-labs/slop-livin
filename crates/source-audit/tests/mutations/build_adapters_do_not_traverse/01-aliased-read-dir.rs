//! target: crates/core/src/build_adapters/node.rs
//! why: alias/rename variant -- `use std::fs::read_dir as list_dir` walks node_modules past a token match
use std::fs::read_dir as list_dir;
pub fn sweep_list(p: &std::path::Path) -> usize {
    list_dir(p).map(|r| r.count()).unwrap_or(0)
}
