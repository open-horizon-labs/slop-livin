//! target: crates/cli/src/main.rs
//! why: authorization minted from a call site that is not in the reviewed allow-list
pub fn sweep_mint_from_anywhere(dir: &std::path::Path) {
    let _ = swamp_core::actions::approve(dir, "plan", "agent");
}
