//! target: crates/core/src/build_adapters/maven.rs
//! why: a hand-rolled walker over ~/.m2/repository is the second full traversal the epic forbids
pub fn sweep_walk(root: &std::path::Path) -> u64 {
    let mut stack = vec![root.to_path_buf()];
    let mut n = 0u64;
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                n += 1;
                stack.push(e.path());
            }
        }
    }
    n
}
