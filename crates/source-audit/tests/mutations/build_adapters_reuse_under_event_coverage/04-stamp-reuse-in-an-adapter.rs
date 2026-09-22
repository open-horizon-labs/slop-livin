//! target: crates/core/src/build_adapters/node.rs
//! why: the stamp-only replay decision moved out of the shared model into an adapter, which a mod.rs-only rule never read
pub fn sweep_replay_if_same(prev: &crate::artifact::NestedArtifact, mtime_now: u64) -> bool {
    prev.mtime_max == mtime_now
}
