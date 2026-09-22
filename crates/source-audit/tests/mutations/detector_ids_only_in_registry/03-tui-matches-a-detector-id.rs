//! target: crates/tui/src/units.rs
//! why: the same coupling in the TUI, which is the surface that most often grows a special case
pub fn sweep_tui_icon(id: &str) -> &'static str {
    match id {
        swamp_core::locations::RUSTUP_DETECTOR_ID => "rust",
        _ => "",
    }
}
