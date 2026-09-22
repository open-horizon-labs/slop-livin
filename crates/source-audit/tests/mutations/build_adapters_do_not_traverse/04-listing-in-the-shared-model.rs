//! target: crates/core/src/build_adapters/mod.rs
//! why: a BuildCtx method in the shared model listing a directory itself; adapters call it as a method, and the old rules exempted mod.rs
impl BuildCtx<'_> {
    pub fn list_everything(&self, dir: &Path) -> usize {
        std::fs::read_dir(dir).map(|r| r.count()).unwrap_or(0)
    }
}
