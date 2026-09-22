//! target: crates/core/src/build_stores.rs
//! why: every store handed to whichever adapter comes first, no capability asked of either side
pub fn sweep_first_adapter(
    adapters: &crate::build_adapters::registry::Registry,
    paths: &[std::path::PathBuf],
) -> Vec<crate::build_adapters::BuildContainer> {
    let Some(first) = adapters.adapters().first() else {
        return Vec::new();
    };
    paths
        .iter()
        .map(|p| crate::build_adapters::BuildContainer::shared_store(first.id(), p.clone()))
        .collect()
}
