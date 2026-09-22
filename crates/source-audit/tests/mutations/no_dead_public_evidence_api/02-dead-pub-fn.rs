//! target: crates/core/src/activity.rs
//! why: a public evidence function with no caller at all
pub fn sweep_dead_evidence_fn(observed_at: u64) -> u64 {
    observed_at
}
