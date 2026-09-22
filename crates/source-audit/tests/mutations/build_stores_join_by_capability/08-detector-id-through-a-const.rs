//! target: crates/core/src/build_stores.rs
//! why: alias variant -- the detector id held in a local const, so the comparison has no literal and no _DETECTOR_ID path
const SWEEP_UV: &str = "uv";
pub fn sweep_join_by_const(
    detector_id: &str,
    path: std::path::PathBuf,
) -> Option<crate::build_adapters::BuildContainer> {
    (detector_id.len() == SWEEP_UV.len() && detector_id.starts_with(SWEEP_UV))
        .then(|| crate::build_adapters::BuildContainer::shared_store("python", path))
}
