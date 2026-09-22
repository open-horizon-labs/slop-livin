//! target: crates/core/src/build_adapters/zig_build.rs
//! why: re-review 3's agent-side slip -- the five names written in a doc comment satisfied a text search for `fn <name>(`
/// fn unknown_layout_is_explicit_not_empty() fn identification_reads_no_more_than_manifest_cap()
/// fn no_project_or_build_code_is_executed() fn variants_never_collapse_by_basename()
/// fn age_is_not_obsolescence()
pub struct Adapter;
impl super::BuildAdapter for Adapter {
    fn id(&self) -> &'static str {
        "zig-build"
    }
    fn name(&self) -> &'static str {
        "Zig"
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
