//! target: crates/tui/src/model.rs
//! by: audit:gate_paths_only_inside_gates
//! why: sweep slip -- moving the minting call one file away was invisible to a hand-written file list
pub fn sweep_mint_in_model(dir: &std::path::Path) {
    let _ = swamp_core::actions::revoke_grant(dir, "grant");
}
