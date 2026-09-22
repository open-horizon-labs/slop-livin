//! target: crates/core/src/reclaimability.rs
//! why: a pub const with no reader -- the ACTIVITY_EVIDENCE_INVENTORY shape
pub const SWEEP_DEAD_INVENTORY: &[(&str, &str)] = &[("a", "b")];
