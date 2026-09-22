//! target: crates/core/src/build_stores.rs
//! why: discarded-result variant -- store_kinds asked and thrown away, then a path suffix decides
pub fn sweep_discarded_kinds(
    adapters: &crate::build_adapters::registry::Registry,
    detectors: &crate::locations::Registry,
    path: std::path::PathBuf,
) -> Vec<crate::build_adapters::BuildContainer> {
    let mut out = Vec::new();
    for d in detectors.detectors() {
        let _ = d.build_stores();
    }
    for a in adapters.adapters() {
        let _ = a.store_kinds();
        if path.ends_with("repository") {
            out.push(crate::build_adapters::BuildContainer::shared_store(a.id(), path.clone()));
        }
    }
    out
}
