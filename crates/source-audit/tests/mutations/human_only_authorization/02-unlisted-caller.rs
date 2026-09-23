//! target: crates/cli/src/main.rs
//! by: compile:E0425
//! note: 2026-09-22 -- the string-actor `approve` exists only under the `testing` feature; see 05/06
//! why: authorization minted from a call site that is not in the reviewed allow-list
pub fn sweep_mint_from_anywhere(dir: &std::path::Path) {
    let _ = swamp_core::actions::approve(dir, "plan", "agent");
}
