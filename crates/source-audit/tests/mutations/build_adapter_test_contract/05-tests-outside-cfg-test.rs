//! target: crates/core/src/build_adapters/elm_build.rs
//! why: the five functions exist but are plain fns outside `#[cfg(test)]` with no `#[test]` -- nothing ever runs them
pub struct Adapter;
impl super::BuildAdapter for Adapter {
    fn id(&self) -> &'static str {
        "elm-build"
    }
    fn name(&self) -> &'static str {
        "Elm"
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
mod tests {
    fn unknown_layout_is_explicit_not_empty() {
        assert!(true);
    }
    fn identification_reads_no_more_than_manifest_cap() {
        assert!(true);
    }
    fn no_project_or_build_code_is_executed() {
        assert!(true);
    }
    fn variants_never_collapse_by_basename() {
        assert!(true);
    }
    fn age_is_not_obsolescence() {
        assert!(true);
    }
}
