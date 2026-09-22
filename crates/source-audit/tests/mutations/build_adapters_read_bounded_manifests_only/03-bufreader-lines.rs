//! target: crates/core/src/build_adapters/gradle.rs
//! why: a line reader over an arbitrary build-directory file is an unbounded read written differently
pub fn sweep_lines(p: &std::path::Path) -> usize {
    use std::io::BufRead;
    let f = std::fs::File::open(p).unwrap();
    std::io::BufReader::new(f).lines().count()
}
