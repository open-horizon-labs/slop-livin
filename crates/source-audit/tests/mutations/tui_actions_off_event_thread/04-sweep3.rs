//! target: crates/tui/src/app.rs
//! mode: append
//! by: compile:E0603
//! why: re-review 3 sweep -- an `lsof` spawn per keystroke, hidden inside `vec![..]` (blind spot: syn::visit never descends into macro tokens, so any sink inside a macro invocation is invisible to the call graph)
impl App {
    /// Sweep: a blocking lsof probe on the key path, inside a macro.
    pub fn handle_key_occupancy(&mut self, p: &std::path::Path) {
        let _states = vec![swamp_core::occupancy::probe_path(p)];
    }
}
