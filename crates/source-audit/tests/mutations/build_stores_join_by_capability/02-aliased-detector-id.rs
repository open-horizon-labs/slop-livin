//! target: crates/core/src/build_stores.rs
//! why: alias/rename variant -- the detector id constant imported under another name
pub fn sweep_join_by_alias(
    detector_id: &str,
    path: std::path::PathBuf,
) -> Option<crate::build_adapters::BuildContainer> {
    use crate::locations::npm::NPM_DETECTOR_ID as STORE_OWNER;
    (detector_id == STORE_OWNER)
        .then(|| crate::build_adapters::BuildContainer::shared_store("node", path))
}
