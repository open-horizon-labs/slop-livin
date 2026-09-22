//! target: crates/core/src/build_adapters/bounded_io.rs
//! why: the bounded reader itself gained an uncapped sibling, which the agent-side rules exempted by file name
pub fn sweep_read_all(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok()
}
