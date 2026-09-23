//! target: crates/core/src/cargo_cleanup.rs
//! by: audit:gate_paths_only_inside_gates
//! source: corpus, 2026-09-22
//! ported: 2026-09-23 -- per-unit TUI confirmations, spent by value (re-review 5, finding 2)
//! why: a sink turning a confirmation it minted into an authorization, skipping the grant
fn sweep_self_authorize(anchor: &std::path::Path) -> Option<crate::authority::Authorized> {
    let mut confirmed = crate::authority::HumanConfirmed::tui_dialog(
        "agent",
        &crate::fs_gate::StoreDir::resolved(),
        Vec::new(),
    );
    crate::authority::authorize_confirmed(confirmed.pop()?, anchor).ok()
}
