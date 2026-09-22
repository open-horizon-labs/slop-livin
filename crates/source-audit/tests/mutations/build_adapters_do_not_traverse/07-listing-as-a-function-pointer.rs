//! target: crates/core/src/build_adapters/maven.rs
//! why: a primitive taken as a value and called through a local binding is never a call edge -- the reference itself is the violation
pub fn sweep_pointer(p: &std::path::Path) -> usize {
    let list = std::fs::read_dir;
    list(p).map(|r| r.count()).unwrap_or(0)
}
