//! target: crates/core/src/actions.rs
//! mode: append
//! expect: reject
//! by: audit:gate_paths_only_inside_gates
//! ported: 2026-09-23 -- confirmations name their subject and are pinned to their handler (re-review 5, finding 2); earlier 2026-09-22 -- granting takes a `HumanConfirmed`; the reviewer's wrapper in actions.rs now has to mint one, outside the reviewed confirmation sites
//! source: review-4 sweep (M group, reviewer_counterexamples_stack4_sweep.rs), M14
//! why: a standing grant minted from the TUI through a new wrapper in the sinks' own file
//! blind-spot (old model): functions in the sinks' file are skipped wholesale ("wiring"), and a caller is checked only for *direct* callees in the sink set, so one hop through `actions.rs` launders any caller
/// Sweep 4: a convenience grant, in the file the audit treats as wiring.
pub fn sweep4_quick_grant(dir: &Path) -> Result<Grant> {
    let terms = crate::authority::StandingTerms {
        predicate: "agent:*".into(),
        budget_bytes: u64::MAX,
        max_units: None,
        expires_in_secs: 1,
    };
    let confirmed = HumanConfirmed::cli_grant("agent", terms);
    add_standing_grant_confirmed(dir, "agent:*", u64::MAX, None, 1, confirmed)
}
//! file: crates/tui/src/actions.rs
//! mode: append
/// Sweep 4: one keystroke, a standing grant, no confirmation.
fn sweep4_on_key_trust(dir: &std::path::Path) {
    let _ = swamp_core::actions::sweep4_quick_grant(dir);
}
