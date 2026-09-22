//! target: crates/core/src/consumer_wiring.rs
//! why: the same defect written differently -- the constructor is called, with an empty reason
pub fn sweep_unavailable_with_no_reason(kind: crate::evidence::FactKind) -> crate::evidence::Evidence {
    crate::evidence::Evidence::unavailable(kind, "")
}
