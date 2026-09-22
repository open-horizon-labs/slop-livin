//! target: crates/core/src/build_adapters/mod.rs
//! why: the spawn moved into a BuildCtx method in the shared model, which the old rules exempted by name; adapters reach it as `ctx.probe(..)`
impl BuildCtx<'_> {
    pub fn probe(&self, dir: &Path) -> bool {
        std::process::Command::new("gradle")
            .arg("properties")
            .current_dir(dir)
            .status()
            .is_ok()
    }
}
