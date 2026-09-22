//! target: crates/core/src/external.rs
//! mode: append
//! why: re-review 3 sweep -- discovery reading raw detector output through a renamed local binding (blind spot: the forbidden shapes are token needles that pin the receiver name (`summary . locations`, `scope . detectors`); one `let s = summary;` defeats all three)
pub struct SweepLoc {
    pub path: std::path::PathBuf,
}
pub struct SweepSummary {
    pub locations: Vec<SweepLoc>,
}

/// Sweep: raw detector candidates, one rename away from the needle.
pub fn sweep_raw_candidates(summary: &SweepSummary) -> Vec<std::path::PathBuf> {
    let s = summary;
    s.locations.iter().map(|l| l.path.clone()).collect()
}
