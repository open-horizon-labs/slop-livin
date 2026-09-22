//! target: crates/core/src/scan.rs
//! mode: replace
//! why: roots no longer canonicalized at the boundary, so `~/src` and its `/private` spelling become two different roots
use std::path::PathBuf;

pub fn roots_from(config: &[String]) -> Vec<PathBuf> {
    config.iter().map(PathBuf::from).collect()
}
