//! target: crates/core/src/build_adapters/mod.rs
//! mode: replace
//! why: discarded-gate variant -- the shared model stops consulting EventCoverage entirely and reuse becomes unconditional
pub struct NestedUnitBuilder;
pub trait BuildAdapter {
    fn id(&self) -> &'static str;
}
pub fn reuse_is_always_allowed() -> bool {
    true
}
