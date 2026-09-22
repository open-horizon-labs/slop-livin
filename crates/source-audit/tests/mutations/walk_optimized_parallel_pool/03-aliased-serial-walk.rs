//! target: crates/core/src/consumers/assemble.rs
//! why: alias/rename variant -- `use crate::attribution::attribute as fold_tree`
use crate::attribution::attribute as fold_tree;

pub fn sweep_aliased_serial(root: &std::path::Path) {
    let _ = fold_tree(root, &[], 1_000);
}
