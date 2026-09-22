//! target: crates/core/src/agents/oh_my_pi.rs
//! why: BufReader::new(File..).lines() reads past the header cap
pub fn sweep_lines(p: &std::path::Path) -> usize {
    use std::io::BufRead;
    let f = std::fs::File::open(p).unwrap();
    std::io::BufReader::new(f).lines().count()
}
