//! target: crates/tui/src/actions.rs
//! why: alias/rename variant -- `use std::process::Command as Proc` hides the constructor from a token match
use std::process::Command as Proc;
pub fn sweep_du(p: &std::path::Path) -> bool {
    Proc::new("du").arg(p).status().is_ok()
}
