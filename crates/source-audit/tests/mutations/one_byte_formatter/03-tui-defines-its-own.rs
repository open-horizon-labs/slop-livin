//! target: crates/tui/src/model.rs
//! why: the TUI must re-export core's formatter, never define one
pub fn human_bytes(n: u64) -> String {
    format!("{n} B")
}
