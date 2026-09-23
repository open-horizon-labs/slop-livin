//! target: crates/tui/src/actions.rs
//! by: compile:E0432
//! note: 2026-09-22 -- the string-actor `add_standing_grant` exists only under the `testing` feature; see 05/06 for the confirmation-token shortcut
//! why: sweep slip -- `use swamp_core::actions::add_standing_grant as mint_standing_grant` in TUI authorize
use swamp_core::actions::add_standing_grant as mint_standing_grant;
pub fn sweep_authorize_mutation(dir: &std::path::Path) {
    let _ = mint_standing_grant(dir, "agent", 0);
}
