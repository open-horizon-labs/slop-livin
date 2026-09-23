//! `ReportAssembled` → (side effect): caches the report so a surface can
//! paint the last known truth instantly. Written only when this run
//! observed, so the cache never runs ahead of the store.

use crate::bus::{Consumer, Ctx, Event, EventKind};
use anyhow::Result;

pub struct CacheWriter;

#[async_trait::async_trait(?Send)]
impl Consumer for CacheWriter {
    fn name(&self) -> &str {
        "cache"
    }
    fn subscribes_to(&self) -> &[EventKind] {
        &[EventKind::ReportAssembled]
    }
    async fn on_event(
        &self,
        event: &Event,
        ctx: &Ctx<'_>,
        _stage: &crate::bus::Stage,
    ) -> Result<Vec<Event>> {
        if let Event::ReportAssembled(report) = event
            && ctx.observe
            && let Some(dir) = &ctx.store_dir
        {
            crate::report::write_last_report(dir, report)?;
        }
        Ok(vec![Event::ReportCached])
    }
}
