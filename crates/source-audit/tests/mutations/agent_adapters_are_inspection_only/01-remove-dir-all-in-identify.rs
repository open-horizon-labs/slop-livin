//! target: crates/core/src/agents/claude_code.rs
//! why: sweep slip -- `fs::remove_dir_all(home.join("logs"))` inside identify: identification deletes data
pub fn sweep_identify_mutation(home: &std::path::Path) {
    let _ = std::fs::remove_dir_all(home.join("logs"));
}
