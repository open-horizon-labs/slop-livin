//! target: crates/core/src/actions.rs
//! expect: accept
//! source: accept, corpus execution_sinks_recheck_live_state/04 ported to the gate
//! ported: 2026-09-23 -- the recheck takes its inputs from the authorization (re-review 5, finding 3)
//! why: the legitimate sink shape -- a fresh recheck proof and an authorization, spent on one Trash move
/// Accept: recheck, then move exactly what was rechecked.
fn sweep_accept_sink(auth: &Authorized, trash: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
    let proof = crate::recheck::run_all(auth)?;
    Ok(fs_gate::destroy::trash_move(proof, auth, trash, "sweep-accept")?.into_path())
}
