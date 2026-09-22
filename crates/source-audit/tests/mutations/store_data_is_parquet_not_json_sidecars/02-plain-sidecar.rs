//! target: crates/core/src/external.rs
//! why: a per-row JSON cache under the store is a parallel database
pub fn sweep_plain_sidecar(swamp_dir: &std::path::Path, bytes: &[u8]) {
    let _ = std::fs::write(swamp_dir.join("identities_cache.json"), bytes);
}
