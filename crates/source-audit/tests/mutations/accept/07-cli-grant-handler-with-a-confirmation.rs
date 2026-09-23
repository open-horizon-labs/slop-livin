//! target: crates/cli/src/main.rs
//! mode: substitute
//! find: let g = swamp_core::actions::add_standing_grant_confirmed(\n        dir,\n        predicate,\n        budget_bytes,\n        max_units,\n        expires_secs,\n        confirmed,\n    )?;
//! expect: accept
//! source: accept
//! ported: 2026-09-23 -- a confirmation is minted only in its handler (re-review 5, finding 2), so the legitimate shape is the reviewed `cmd_grant_add` handing its confirmation to a helper; minting one in the helper is `token_binding_and_gate_hardening/18`
//! why: `swamp grant add` with the grant call extracted into a helper beside its handler, the confirmation minted where the human's terms were read
let g = sweep_accept_grant(dir, predicate, budget_bytes, max_units, expires_secs, confirmed)?;
//! file: crates/cli/src/main.rs
//! mode: append
/// Accept: the grant call, extracted from `cmd_grant_add`.
fn sweep_accept_grant(
    dir: &Path,
    predicate: &str,
    budget_bytes: u64,
    max_units: Option<u32>,
    expires_secs: u64,
    confirmed: swamp_core::authority::HumanConfirmed,
) -> Result<swamp_core::actions::Grant> {
    swamp_core::actions::add_standing_grant_confirmed(
        dir,
        predicate,
        budget_bytes,
        max_units,
        expires_secs,
        confirmed,
    )
}
