//! target: crates/core/src/cargo_cleanup.rs
//! by: audit:gate_paths_only_inside_gates
//! source: corpus, 2026-09-22
//! why: a sink turning a confirmation it minted into an authorization, skipping the grant
fn sweep_self_authorize(anchors: &[std::path::PathBuf]) -> Option<crate::authority::Authorized> {
    let confirmed = crate::authority::HumanConfirmed::tui_dialog("agent");
    crate::authority::authorize_confirmed(&confirmed, "self", anchors)
}
