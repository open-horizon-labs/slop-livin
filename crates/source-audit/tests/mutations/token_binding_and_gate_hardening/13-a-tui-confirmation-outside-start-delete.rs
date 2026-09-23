//! target: crates/tui/src/app.rs
//! mode: append
//! by: audit:gate_paths_only_inside_gates
//! source: re-review 5, finding 2
//! why: a key handler minting TUI confirmations: `tui_dialog` is pinned to `start_delete`, the Enter on the summary the human just read
impl App {
    fn sweep5_confirm_on_key(&mut self) {
        let _ = swamp_core::authority::HumanConfirmed::tui_dialog(
            &self.actor,
            &swamp_core::fs_gate::StoreDir::resolved(),
            Vec::new(),
        );
    }
}
