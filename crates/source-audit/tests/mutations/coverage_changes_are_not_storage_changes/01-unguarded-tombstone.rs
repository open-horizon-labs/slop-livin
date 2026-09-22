//! target: crates/core/src/external.rs
//! why: sweep slip -- a `present = false` sweep with no ObservationOwnership guard in the same function
pub fn sweep_mark_missing_external(rows: &mut [crate::growth::StoredExternalRowPublic]) {
    for row in rows.iter_mut() {
        row.present = false;
    }
}
