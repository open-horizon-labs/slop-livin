use crate::entities::{Confidence, FactMeta};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RefreshRefusal {
    NoStoredEventId,
    EventIdFromFuture,
    HorizonExceeded,
    UnsupportedPlatform,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshResult {
    pub incremental: bool,
    pub refusal: Option<RefreshRefusal>,
    pub meta: FactMeta,
}
pub fn full_refresh(reason: RefreshRefusal) -> RefreshResult {
    RefreshResult {
        incremental: false,
        refusal: Some(reason),
        meta: FactMeta::now("filesystem.full-refresh", Confidence::High),
    }
}
