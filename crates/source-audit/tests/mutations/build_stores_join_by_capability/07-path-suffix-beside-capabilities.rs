//! target: crates/core/src/build_stores.rs
//! why: both capabilities honoured, and a path-suffix test still overrides them (a custom GRADLE_USER_HOME defeats it)
pub fn sweep_suffix_override(
    adapters: &crate::build_adapters::registry::Registry,
    detectors: &crate::locations::Registry,
    path: std::path::PathBuf,
) -> Vec<crate::build_adapters::BuildContainer> {
    let declared: usize = detectors.detectors().iter().map(|d| d.build_stores().len()).sum();
    let mut out = Vec::new();
    for a in adapters.adapters() {
        if !a.store_kinds().is_empty() && declared > 0 && path.to_string_lossy().ends_with("/caches") {
            out.push(crate::build_adapters::BuildContainer::shared_store(a.id(), path.clone()));
        }
    }
    out
}
