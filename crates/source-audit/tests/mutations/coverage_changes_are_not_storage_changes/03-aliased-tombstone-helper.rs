//! target: crates/core/src/agents/mod.rs
//! why: alias/rename variant -- the tombstone written through a renamed local binding, one function away from any guard
pub fn sweep_tombstone_agent_rows(rows: &mut Vec<crate::growth::StoredExternalRowPublic>) {
    for r in rows.iter_mut() {
        let present = &mut r.present;
        *present = false;
    }
}
