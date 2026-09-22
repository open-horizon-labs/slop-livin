//! target: crates/core/src/build_adapters/maven.rs
//! why: discarded-result variant -- the bytes are still read off disk even when the value is thrown away
pub fn sweep_touch(p: &std::path::Path) -> bool {
    let _ = std::fs::read(p);
    true
}
