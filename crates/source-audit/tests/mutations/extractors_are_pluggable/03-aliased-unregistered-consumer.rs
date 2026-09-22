//! target: crates/core/src/consumers/projects.rs
//! why: alias/rename variant -- the trait imported under another name, so a "impl Consumer for" needle would miss it
use crate::bus::Consumer as SweepStage;

pub struct SweepAliasedConsumer;

impl SweepStage for SweepAliasedConsumer {
    fn name(&self) -> &'static str {
        "sweep-aliased"
    }
}
