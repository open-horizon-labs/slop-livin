//! target: crates/core/src/build_adapters/gradle.rs
//! why: discarded-result variant -- `let _ = remove_dir_all(..)` deletes the user's build cache inside identification
pub fn sweep_prune(dir: &std::path::Path) {
    let _ = std::fs::remove_dir_all(dir.join("build"));
}
