//! target: crates/core/src/report.rs
//! why: populated, but `skip_serializing_if` hides it and no delivery file reads it -- computed and not delivered
#[derive(serde::Serialize)]
pub struct SweepHiddenEvidenceRow {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub evidence_sweep_hidden: Vec<crate::evidence::Evidence>,
}

pub fn sweep_build_hidden_row(e: Vec<crate::evidence::Evidence>) -> SweepHiddenEvidenceRow {
    SweepHiddenEvidenceRow {
        evidence_sweep_hidden: e,
    }
}
