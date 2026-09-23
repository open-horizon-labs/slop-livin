//! target: crates/core/src/recheck.rs
//! mode: append
//! expect: reject
//! source: review-4 sweep (M group, reviewer_counterexamples_stack4_sweep.rs), M9
//! by: compile:clippy::disallowed_methods
//! why: a reviewed member sized through `Path::metadata()` (the method), which follows symlinks
//! blind-spot (old model): a following stat is the free function `fs::metadata` only; `p.metadata()`, `File::open(p)?.metadata()` and `Path::metadata(p)` follow the link and are not calls to it
/// Sweep 4: stats through the link, spelled as a method.
pub fn sweep4_member_size(p: &Path) -> u64 {
    p.metadata().map(|m| m.len()).unwrap_or(0)
}
