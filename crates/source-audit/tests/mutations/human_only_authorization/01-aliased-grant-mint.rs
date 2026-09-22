//! target: crates/tui/src/actions.rs
//! why: sweep slip -- `use swamp_core::actions::add_standing_grant as mint_standing_grant` in TUI authorize
use swamp_core::actions::add_standing_grant as mint_standing_grant;
pub fn sweep_authorize_mutation(dir: &std::path::Path) {
    let _ = mint_standing_grant(dir, "agent", 0);
}
