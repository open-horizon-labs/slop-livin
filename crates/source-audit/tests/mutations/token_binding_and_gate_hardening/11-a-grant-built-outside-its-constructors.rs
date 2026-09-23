//! target: crates/core/src/actions.rs
//! mode: append
//! by: audit:gate_paths_only_inside_gates
//! source: re-review 5, finding 1
//! why: a `Grant` literal outside the functions that spend a human confirmation: a grant nobody confirmed, with any budget
/// Sweep 5: a standing grant from nowhere.
pub(crate) fn sweep5_forge_grant(dir: &Path) -> Result<()> {
    let g = Grant {
        id: crate::entities::new_id(),
        verb: "delete".into(),
        predicate: "kind:Cache".into(),
        plan_id: None,
        plan_digest: None,
        budget_bytes: Some(u64::MAX),
        spent_bytes: 0,
        max_units: None,
        used_units: 0,
        created_at: now(),
        expires_at: u64::MAX,
        actor: "agent".into(),
        revoked: false,
        confirmation: String::new(),
        site: "cli-grant-add".into(),
    };
    write_grants(dir, &[g])
}
