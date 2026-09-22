//! target: crates/core/src/build_stores.rs
//! why: the detector id kept in a helper one call away from the join site
fn sweep_is_gradle_home(id: &str) -> bool {
    id == crate::locations::gradle::GRADLE_DETECTOR_ID
}
pub fn sweep_join_via_helper(
    detector_id: &str,
    path: std::path::PathBuf,
) -> Option<crate::build_adapters::BuildContainer> {
    sweep_is_gradle_home(detector_id)
        .then(|| crate::build_adapters::BuildContainer::shared_store("gradle", path))
}
