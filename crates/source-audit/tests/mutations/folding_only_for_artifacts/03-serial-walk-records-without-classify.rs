//! target: crates/core/src/attribution.rs
//! why: the serial walk's half of the same rule -- an artifact row recorded with no classification behind it
pub fn sweep_record_any_dir(path: &std::path::Path, shared: &mut Vec<String>) {
    record_artifact(path, shared);
}
