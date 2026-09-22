//! target: crates/core/src/consumers/extra_build.rs
//! why: an adapter defined outside build_adapters/ escaped every rule that listed that directory
pub struct Adapter;
impl crate::build_adapters::BuildAdapter for Adapter {
    fn id(&self) -> &'static str {
        "extra"
    }
    fn name(&self) -> &'static str {
        "Extra"
    }
    fn containers(
        &self,
        _root: &std::path::Path,
        _candidates: &[std::path::PathBuf],
    ) -> Vec<crate::build_adapters::BuildContainer> {
        Vec::new()
    }
    fn identify(
        &self,
        _c: &crate::build_adapters::BuildContainer,
        _ctx: &crate::build_adapters::BuildCtx,
    ) -> Vec<crate::artifact::NestedArtifact> {
        Vec::new()
    }
}
