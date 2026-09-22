//! target: crates/core/src/agents/mod.rs
//! why: the same store as .jsonl rather than .json
pub fn sweep_jsonl_sidecar(swamp_dir: &std::path::Path, bytes: &[u8]) {
    let _ = std::fs::write(swamp_dir.join("session_index.jsonl"), bytes);
}
