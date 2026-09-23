//! target: crates/core/src/actions.rs
//! expect: accept
//! source: accept, corpus execution_sinks_recheck_live_state/04 ported to the gate
//! why: the legitimate sink shape -- a fresh recheck proof and an authorization, spent on one Trash move
/// Accept: recheck, then move exactly what was rechecked.
fn sweep_accept_sink(
    store: &std::path::Path,
    path: &std::path::Path,
    auth: &Authorized,
    trash: &std::path::Path,
) -> anyhow::Result<std::path::PathBuf> {
    let proof = crate::recheck::run_all(store, path, None, &[])?;
    fs_gate::destroy::trash_move(proof, auth, trash, "sweep-accept")
}
