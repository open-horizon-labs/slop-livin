//! target: crates/core/src/build_adapters/gradle.rs
//! why: the listing lives in the ecosystem catalog (`detect` lists a directory) and the adapter only calls it
pub fn sweep_detect(p: &std::path::Path) -> usize {
    crate::ecosystem::detect(p).len()
}
