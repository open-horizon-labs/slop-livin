//! target: crates/core/src/build_adapters/gradle.rs
//! why: discarded-result variant -- the coverage gate is called and its answer thrown away before replaying the cache
pub fn sweep_replay(ctx_cov: &crate::fs_events::EventCoverage, cache: &super::ContainerCache, p: &std::path::Path) -> usize {
    let _ = ctx_cov.unchanged_since(p, 0);
    cache.entries.borrow().len()
}
