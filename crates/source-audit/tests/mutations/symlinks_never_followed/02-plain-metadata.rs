//! target: crates/core/src/attribution.rs
//! why: the original forbidden shape must still be rejected
pub fn sweep_follow_link_plain(p: &std::path::Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}
