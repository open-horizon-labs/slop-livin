//! target: crates/core/src/activity.rs
//! why: an Activity fact with no EvidenceSource -- "when was this last used" with nothing saying how we know
pub fn sweep_last_used_evidence(at: u64) -> crate::evidence::Evidence {
    let mut e = crate::evidence::Evidence::default();
    e.observed_at = at;
    e
}
