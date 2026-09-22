//! target: crates/core/src/build_adapters/node.rs
//! why: re-review 3 slip -- `std::io::read_to_string(File::open(p)?)`, the free function a list holding only `fs::read_to_string` missed
pub fn sweep_io(p: &std::path::Path) -> std::io::Result<String> {
    std::io::read_to_string(std::fs::File::open(p)?)
}
