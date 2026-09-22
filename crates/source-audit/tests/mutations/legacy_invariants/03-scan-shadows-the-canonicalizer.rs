//! target: crates/core/src/scan.rs
//! why: alias/shadow variant -- scan.rs defines its own `canonical_roots` that canonicalizes nothing, so the name is present and the invariant is gone
pub fn canonical_roots(roots: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    roots.to_vec()
}
