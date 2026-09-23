//! target: crates/core/src/render.rs
//! mode: append
//! by: audit:gate_paths_only_inside_gates
//! source: re-review 5, finding 6
//! why: `include!` of a computed path (a build script's `OUT_DIR`): its target cannot be followed, so what it splices in is outside every rule
include!(concat!(env!("OUT_DIR"), "/sweep5_generated.rs"));
