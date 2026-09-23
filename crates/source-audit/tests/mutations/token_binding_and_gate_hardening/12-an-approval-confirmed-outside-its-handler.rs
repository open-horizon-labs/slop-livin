//! target: crates/cli/src/main.rs
//! mode: append
//! by: audit:gate_paths_only_inside_gates
//! source: re-review 5, finding 2
//! why: a CLI helper that confirms and approves every stored plan: `cli_approve` is pinned to `cmd_approve`, the one function that has just shown the human the plan
/// Sweep 5: approve everything, no one watching.
fn sweep5_approve_all() -> Result<()> {
    for p in swamp_core::actions::list_plans(&swamp_dir())? {
        let confirmed = swamp_core::authority::HumanConfirmed::cli_approve("agent", &p);
        swamp_core::actions::approve_confirmed(&swamp_dir(), &p.id, confirmed)?;
    }
    Ok(())
}
