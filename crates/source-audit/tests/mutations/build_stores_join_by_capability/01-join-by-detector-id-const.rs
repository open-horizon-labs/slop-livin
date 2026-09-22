//! target: crates/core/src/build_stores.rs
//! why: the table the capability replaces -- a store handed to Maven because its detector id is Maven's
pub fn sweep_join_by_id(
    detector_id: &str,
    path: std::path::PathBuf,
) -> Option<crate::build_adapters::BuildContainer> {
    (detector_id == crate::locations::maven::MAVEN_DETECTOR_ID)
        .then(|| crate::build_adapters::BuildContainer::shared_store("maven", path))
}
