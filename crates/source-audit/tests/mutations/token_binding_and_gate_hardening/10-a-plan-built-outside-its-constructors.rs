//! target: crates/core/src/actions.rs
//! mode: append
//! by: audit:gate_paths_only_inside_gates
//! source: re-review 5, finding 1
//! why: a `Plan` literal in a helper inside `actions` (where privacy cannot stop it): a plan is built only by a propose path or by the loader that verified its binding
/// Sweep 5: a plan nobody proposed.
pub(crate) fn sweep5_forge_plan(units: Vec<PlanUnit>) -> Plan {
    Plan {
        id: crate::entities::new_id(),
        root: PathBuf::new(),
        created_at: 0,
        expires_at: u64::MAX,
        proposed_by: "agent".into(),
        status: PlanStatus::Proposed,
        units,
        refused: Vec::new(),
        selection: None,
        digest: std::sync::OnceLock::new(),
    }
}
