//! target: crates/tui/src/app.rs
//! by: compile:E0603
//! why: sweep slip -- `probe_path` (an lsof spawn) per keystroke was not in the blocking sink list
impl App {
    pub fn sweep_on_key_blocking(&mut self, path: &std::path::Path) {
        let _ = swamp_core::occupancy::probe_path(path);
    }
}
