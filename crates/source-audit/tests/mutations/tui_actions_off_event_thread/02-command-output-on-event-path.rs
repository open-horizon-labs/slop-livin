//! target: crates/tui/src/app.rs
//! why: sweep slip -- Command::output was not in the blocking sink list
impl App {
    pub fn sweep_handle_key_spawn(&mut self) {
        let _ = std::process::Command::new("du").arg("-sk").output();
    }
}
