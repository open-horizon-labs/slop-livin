//! target: crates/core/src/report.rs
//! mode: append
//! by: audit:gate_paths_only_inside_gates
//! source: re-review 5, finding 1
//! why: the authority key read outside `actions`/`authority`/`recheck`: whoever holds it can bind a record as if swamp's propose/approve path wrote it
/// Sweep 5: a second binder.
pub(crate) fn sweep5_key(dir: &std::path::Path) -> Option<[u8; 32]> {
    let store = crate::fs_gate::StoreDir::resolved();
    let _ = dir;
    crate::fs_gate::key::authority_key(&store).ok()
}
