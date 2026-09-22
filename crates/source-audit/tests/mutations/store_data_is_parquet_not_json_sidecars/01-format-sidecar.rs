//! target: crates/core/src/growth.rs
//! why: sweep slip -- `format!("project-{}.json")` hides the sidecar's extension from a literal scan
pub fn sweep_json_sidecar(swamp_dir: &std::path::Path, id: u32, bytes: &[u8]) {
    let p = swamp_dir.join(format!("project-{}.json", id));
    let _ = std::fs::write(p, bytes);
}
