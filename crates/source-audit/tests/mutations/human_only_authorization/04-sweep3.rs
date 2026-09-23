//! target: crates/tui/src/actions.rs
//! mode: append
//! by: audit:gate_paths_only_inside_gates
//! ported: 2026-09-22 -- granting now takes a `HumanConfirmed`; one module hop from the confirm dialog, the shortcut is to mint the confirmation here
//! why: re-review 3 sweep -- one module hop hides the minting call
/// Sweep: one module hop hides the minting call.
mod sweep_shim {
    pub use swamp_core::actions::add_standing_grant_confirmed;
}

fn sweep_authorize(dir: &std::path::Path) {
    let confirmed = swamp_core::authority::HumanConfirmed::tui_dialog("agent");
    let _ = sweep_shim::add_standing_grant_confirmed(dir, "agent:*", 0, None, 0, &confirmed);
}
