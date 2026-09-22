//! target: crates/core/src/build_adapters/matrix.rs
//! why: a matrix entry whose actions field names a capability no executor backs -- written through a const so a text match on the field misses it
pub const SWEEP_ACTIONS: &str = "selective cleanup";
pub const SWEEP_EXTRA: &[MatrixEntry] = &[MatrixEntry {
    id: "cargo",
    name: "Rust / Cargo",
    status: Status::Planned,
    families: &[],
    known_layouts: &[],
    attribution_limits: &[],
    operation_granularity: "whole artifact row",
    actions: SWEEP_ACTIONS,
}];
