//! target: crates/tui/src/actions.rs
//! mode: append
//! by: audit:gate_paths_only_inside_gates
//! source: re-review 5, finding 2
//! why: a TUI helper turning a confirmation into an authorization for a path of its choosing: `authorize_confirmed` is pinned to `execute_one`
/// Sweep 5: authorize a path the dialog did not list.
pub fn sweep5_widen(confirmed: HumanConfirmed, path: &Path) -> Result<Authorized> {
    swamp_core::authority::authorize_confirmed(confirmed, path)
}
