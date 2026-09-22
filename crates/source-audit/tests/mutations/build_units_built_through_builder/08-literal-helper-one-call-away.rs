//! target: crates/core/src/build_adapters/cargo.rs
//! why: the struct literal lives in the legacy inspection module and the adapter calls it -- a literal one call away is still a literal
pub fn sweep_legacy(target: &std::path::Path) -> usize {
    crate::cargo_artifacts::inspect_target(target, None).units.len()
}
