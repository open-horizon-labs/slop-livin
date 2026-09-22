//! target: crates/tui/src/actions.rs
//! mode: append
//! why: re-review 3 sweep -- a standing grant minted from the TUI through a local re-export module (blind spot: the rule is `body.contains("actions :: <sink> (")` after one level of *use-alias* resolution; a `pub use` inside a local `mod` is a second hop the resolver does not take)
/// Sweep: one module hop hides the minting call.
pub mod sweep_shim {
    pub use swamp_core::actions::add_standing_grant;
}

pub fn sweep_authorize(dir: &std::path::Path) {
    let _ = sweep_shim::add_standing_grant(dir, "agent:*", 0, None, 0, "agent");
}
