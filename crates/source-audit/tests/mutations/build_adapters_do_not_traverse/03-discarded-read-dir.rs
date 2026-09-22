//! target: crates/core/src/build_adapters/gradle.rs
//! why: discarded-result variant -- the directory is still enumerated when the iterator is dropped
pub fn sweep_warm(p: &std::path::Path) {
    let _ = std::fs::read_dir(p.join("caches"));
}
