//! target: crates/core/src/build_adapters/node.rs
//! why: re-review 3 slip class 5 -- `syn::visit` does not descend into macro tokens, so `vec![read_dir(..)]` hid a listing
pub fn sweep_macro(p: &std::path::Path) -> usize {
    let v = vec![std::fs::read_dir(p)];
    v.len()
}
