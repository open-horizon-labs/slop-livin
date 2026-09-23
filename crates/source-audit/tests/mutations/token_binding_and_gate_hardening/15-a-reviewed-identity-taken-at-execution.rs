//! target: crates/core/src/cargo_cleanup.rs
//! mode: append
//! by: audit:gate_paths_only_inside_gates
//! source: re-review 5, finding 3
//! why: a sink capturing the "reviewed" identity itself, right before the recheck compares against it: identity is recorded when a unit is proposed or marked, never at execution
/// Sweep 5: review what is there now.
pub(crate) fn sweep5_review_now(p: &Path) -> Option<crate::recheck::ReviewedIdentity> {
    crate::recheck::capture_anchor(p).ok()
}
