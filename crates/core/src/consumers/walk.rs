//! `RootRequested` → `RootObserved`: discovery and attribution of the
//! root. With a store, the FSEvents-driven incremental path runs first and
//! falls back to a full walk on refusal; without one, a full walk.

use crate::bus::{Consumer, Ctx, Event, EventKind};
use anyhow::Result;
use std::sync::Arc;

pub struct WalkConsumer;

#[async_trait::async_trait(?Send)]
impl Consumer for WalkConsumer {
    fn name(&self) -> &str {
        "walk"
    }
    fn subscribes_to(&self) -> &[EventKind] {
        &[EventKind::RootRequested]
    }
    async fn on_event(&self, _event: &Event, ctx: &Ctx<'_>) -> Result<Vec<Event>> {
        let mut notes: Vec<String> = Vec::new();
        let mut rewalked: Option<Arc<Vec<String>>> = None;
        let mut changed_paths = None;
        let (discovered, attribution) = if let Some(dir) = &ctx.store_dir {
            let tracked = crate::growth::observe_tracked_with_source(
                dir,
                &ctx.root,
                ctx.observed_at,
                ctx.large_file_min_bytes,
                ctx.force_full,
                ctx.observe,
                ctx.fs_events,
            )?;
            notes.push(format!(
                "fsevents: mode={} reason={} changed_dirs={}",
                tracked.mode, tracked.reason, tracked.changed_dirs
            ));
            rewalked = tracked.rewalked.map(Arc::new);
            changed_paths = tracked.changed_paths.map(Arc::new);
            (tracked.discovered, tracked.attribution)
        } else {
            notes.push("fsevents: mode=full reason=no_store changed_dirs=0".to_string());
            crate::walk::discover_and_attribute(
                &ctx.root,
                ctx.observed_at,
                ctx.large_file_min_bytes,
            )?
        };
        Ok(vec![Event::RootObserved {
            changed_paths,
            discovered: Arc::new(discovered),
            attribution: Arc::new(attribution),
            notes,
            rewalked,
        }])
    }
}
