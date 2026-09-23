//! target: crates/cli/src/main.rs
//! expect: accept
//! source: accept
//! why: a CLI handler minting its own human confirmation and a standing grant with it (the CLI is a reviewed confirmation site)
/// Accept: `swamp grant add` spelled as a helper.
fn sweep_accept_grant(predicate: &str) -> Result<()> {
    let confirmed = swamp_core::authority::HumanConfirmed::cli_command("human:cli");
    swamp_core::actions::add_standing_grant_confirmed(
        &swamp_dir(),
        predicate,
        1 << 20,
        Some(1),
        3600,
        &confirmed,
    )?;
    Ok(())
}
