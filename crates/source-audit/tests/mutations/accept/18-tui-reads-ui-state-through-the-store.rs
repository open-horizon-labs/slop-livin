//! target: crates/tui/src/app.rs
//! expect: accept
//! source: accept
//! why: the TUI reading its remembered state through the typed store file, at startup (not on the event thread)
/// Accept: one of swamp's own files, by name.
fn sweep_accept_ui_state_bytes(store: &std::path::Path) -> usize {
    swamp_core::fs_gate::store::read_json_bytes(swamp_core::fs_gate::store::JsonFile::UiState { store })
        .ok()
        .flatten()
        .map(|b| b.len())
        .unwrap_or(0)
}
