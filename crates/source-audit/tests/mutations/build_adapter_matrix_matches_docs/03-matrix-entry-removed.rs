//! target: crates/core/src/build_adapters/matrix.rs
//! mode: replace
//! why: discarded-entry variant -- the code's capability table silently loses a family the registry still runs
pub struct MatrixEntry {
    pub id: &'static str,
}
pub const MATRIX: &[MatrixEntry] = &[MatrixEntry { id: "cargo" }];
