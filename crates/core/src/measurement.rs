use crate::entities::now;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseStudy {
    pub name: String,
    pub started_at: u64,
    pub duration_days: u32,
    pub free_space_floor_bytes: u64,
    pub minimum_pressure_events: u32,
    pub readings: [String; 3],
    pub proposal_enabled: bool,
}
pub fn preregister(name: String, duration_days: u32, floor: u64, min_events: u32) -> CaseStudy {
    CaseStudy {
        name,
        started_at: now(),
        duration_days,
        free_space_floor_bytes: floor,
        minimum_pressure_events: min_events,
        readings: [
            "works".into(),
            "framing-wrong".into(),
            "inconclusive".into(),
        ],
        proposal_enabled: false,
    }
}
