//! target: crates/core/src/build_adapters/matrix.rs
//! why: a documented family marked implemented with no adapter behind it -- the reverse direction of the registry/matrix equality
pub const SWEEP_PHANTOM: &[MatrixEntry] = &[MatrixEntry {
    id: "bazel",
    name: "Bazel",
    status: Status::Implemented,
    families: &[crate::artifact::RoleFamily::Outputs],
    known_layouts: &["bazel-out/"],
    attribution_limits: &["none"],
    operation_granularity: "whole artifact row",
    actions: INSPECTION_ONLY,
}];
