//! target: crates/core/src/render.rs
//! mode: append
//! by: audit:gate_paths_only_inside_gates
//! source: re-review 5, finding 6
//! why: `include_str!` of a file outside the crate's own `src/`: what a crate splices in must live where every rule reads it
/// Sweep 5: bytes from outside the source tree.
pub(crate) const SWEEP5_SPLICED: &str = include_str!("../../../Cargo.toml");
