//! target: crates/core/src/report.rs
//! why: sweep slip -- `let _owned = ownership.owns(key);` then an unguarded regrowth bump
pub fn sweep_regrow(
    ownership: &crate::growth::ObservationOwnership,
    key: &str,
    row: &mut crate::growth::StoredExternalRowPublic,
) {
    let _owned = ownership_answer(ownership, key);
    row.regrowth_count = row.regrowth_count + 1;
}

fn ownership_answer(_o: &crate::growth::ObservationOwnership, _k: &str) -> bool {
    true
}
