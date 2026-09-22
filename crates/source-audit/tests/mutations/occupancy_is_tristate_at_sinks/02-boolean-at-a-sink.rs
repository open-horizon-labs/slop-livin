//! target: crates/core/src/cargo_cleanup.rs
//! why: the boolean probe whose own doc comment forbids sink use, consumed at a sink
pub fn sweep_boolean_occupancy(p: &std::path::Path) -> bool {
    crate::agents::is_active(p)
}
