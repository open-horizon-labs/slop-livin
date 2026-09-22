//! target: crates/cli/src/main.rs
//! why: the same coupling in the CLI, where a hand-written file list would not look
fn sweep_cli_hint(id: &str) -> &'static str {
    if id == swamp_core::locations::HOMEBREW_DETECTOR_ID {
        "brew cleanup"
    } else {
        ""
    }
}
