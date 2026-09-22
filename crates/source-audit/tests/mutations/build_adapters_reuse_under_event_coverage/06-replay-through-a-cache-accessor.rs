//! target: crates/core/src/build_adapters/mod.rs
//! why: a new accessor on the cache type hands stored units to a caller that replays them with no coverage gate -- the storage field never appears at the replay site
impl ContainerCache {
    pub fn stored(&self, key: &str) -> Vec<NestedArtifact> {
        self.entries.borrow().get(key).map(|(_, u)| u.clone()).unwrap_or_default()
    }
}
pub fn sweep_replay_stored(cache: &ContainerCache, c: &BuildContainer) -> Vec<NestedArtifact> {
    cache.stored(&c.scope())
}
