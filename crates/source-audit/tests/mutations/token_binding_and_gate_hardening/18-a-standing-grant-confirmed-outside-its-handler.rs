//! target: crates/cli/src/main.rs
//! mode: append
//! by: audit:gate_paths_only_inside_gates
//! source: re-review 5, finding 2 (the former accept/07 shape)
//! why: a CLI helper minting its own standing-grant confirmation: `cli_grant` is pinned to `cmd_grant_add`, where the terms the human typed are read
/// Sweep 5: `swamp grant add` spelled as a helper that confirms itself.
fn sweep5_quick_grant(predicate: &str) -> Result<()> {
    let terms = swamp_core::authority::StandingTerms {
        predicate: predicate.to_string(),
        budget_bytes: 1 << 20,
        max_units: Some(1),
        expires_in_secs: 3600,
    };
    let confirmed = swamp_core::authority::HumanConfirmed::cli_grant("human:cli", terms);
    swamp_core::actions::add_standing_grant_confirmed(
        &swamp_dir(),
        predicate,
        1 << 20,
        Some(1),
        3600,
        confirmed,
    )?;
    Ok(())
}
