//! target: crates/core/src/activity.rs
//! why: an Unknown built as a struct literal outside evidence.rs, so nothing makes the reason mandatory
pub fn sweep_unknown_without_reason() -> crate::evidence::FactStatus {
    crate::evidence::FactStatus::Unknown {}
}
