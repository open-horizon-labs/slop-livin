//! target: crates/core/src/build_stores.rs
//! expect: accept
//! why: the sanctioned shape -- the detector says which location is which store kind, the adapter says which kinds it identifies, and the join compares the two declarations and nothing else
pub fn sweep_capability_join(
    adapters: &crate::build_adapters::registry::Registry,
    detectors: &crate::locations::Registry,
    located: &[(String, std::path::PathBuf)],
) -> Vec<crate::build_adapters::BuildContainer> {
    let mut out = Vec::new();
    for (detector, path) in located {
        let Some(d) = detectors.detectors().iter().find(|d| d.id() == detector.as_str()) else {
            continue;
        };
        for decl in d.build_stores() {
            if let Some(a) = adapters
                .adapters()
                .iter()
                .find(|a| a.store_kinds().contains(&decl.kind))
            {
                out.push(crate::build_adapters::BuildContainer::shared_store(a.id(), path.clone()));
            }
        }
    }
    out
}
