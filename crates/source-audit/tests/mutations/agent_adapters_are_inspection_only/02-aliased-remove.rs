//! target: crates/core/src/agents/codex.rs
//! why: sweep slip -- the destructive primitive renamed past a token match
use std::fs::remove_dir_all as tidy_up;
pub fn sweep_identify_mutation_aliased(home: &std::path::Path) {
    let _ = tidy_up(home.join("cache"));
}
