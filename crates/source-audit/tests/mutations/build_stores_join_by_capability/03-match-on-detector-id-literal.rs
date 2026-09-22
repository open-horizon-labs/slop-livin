//! target: crates/core/src/build_stores.rs
//! why: a central match over detector id literals, the shape section 13 removed from the agent side
pub fn sweep_match_ids(
    detector_id: &str,
    path: std::path::PathBuf,
) -> Option<crate::build_adapters::BuildContainer> {
    let adapter = match detector_id {
        "npm" | "pnpm" => "node",
        "gradle" => "gradle",
        _ => return None,
    };
    Some(crate::build_adapters::BuildContainer::shared_store(adapter, path))
}
