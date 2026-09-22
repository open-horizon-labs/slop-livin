//! target: crates/core/src/build_adapters/bazel_build.rs
//! why: alias/rename variant -- `use super::BuildAdapter as Identify; impl Identify for ..` is an adapter with no proofs, and a derivation matching the trait's spelling would not count it
use super::BuildAdapter as Identify;
pub struct Bazel;
impl Identify for Bazel {
    fn id(&self) -> &'static str {
        "bazel-build"
    }
    fn name(&self) -> &'static str {
        "Bazel"
    }
    fn containers(
        &self,
        _root: &std::path::Path,
        _candidates: &[std::path::PathBuf],
    ) -> Vec<super::BuildContainer> {
        Vec::new()
    }
    fn identify(
        &self,
        _c: &super::BuildContainer,
        _ctx: &super::BuildCtx,
    ) -> Vec<crate::artifact::NestedArtifact> {
        Vec::new()
    }
}
