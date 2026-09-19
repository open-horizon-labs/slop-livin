//! Replay-informed Cargo projection over the folded directory measurements.
use crate::bus::{Consumer, Ctx, Draft, Event, EventKind};
use anyhow::Result;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Default)]
pub struct CargoConsumer {
    changed: Mutex<Option<Arc<Vec<PathBuf>>>>,
}

#[async_trait::async_trait(?Send)]
impl Consumer for CargoConsumer {
    fn name(&self) -> &str {
        "cargo-artifacts"
    }
    fn subscribes_to(&self) -> &[EventKind] {
        &[EventKind::RootObserved, EventKind::RowsAssembled]
    }
    async fn on_event(&self, event: &Event, ctx: &Ctx<'_>) -> Result<Vec<Event>> {
        if let Event::RootObserved { changed_paths, .. } = event {
            *self.changed.lock().unwrap() = changed_paths.clone();
            return Ok(vec![]);
        }
        let Event::RowsAssembled(draft) = event else {
            return Ok(vec![]);
        };
        let mut draft: Draft = (**draft).clone();
        // Aggregate once, before either Cargo interpretation or history uses it.
        let artifact_roots = crate::report::artifact_roots(&draft.projects);
        crate::report::aggregate_dir_totals(&mut draft.dirs, &artifact_roots);
        let cached = ctx
            .store_dir
            .as_deref()
            .and_then(|d| crate::report::load_last_report(d, &ctx.root))
            .map(|r| r.nested_artifacts)
            .unwrap_or_default();
        let changed = self.changed.lock().unwrap().clone();
        let roots = crate::cargo_artifacts::project_roots(&draft.projects);
        let mut grouped: std::collections::HashMap<String, Vec<_>> =
            std::collections::HashMap::new();
        for u in cached {
            grouped
                .entry(u.container_id.clone().unwrap_or_else(|| u.id.clone()))
                .or_default()
                .push(u);
        }
        let mut nested = Vec::new();
        for (root, _workspace) in roots {
            let old = grouped
                .remove(&crate::artifact::NestedArtifact::storage_id(&root, ""))
                .unwrap_or_default();
            let valid = old.iter().any(|u| {
                u.path == root
                    && u.id == crate::artifact::NestedArtifact::storage_id(&root, "")
                    && u.coverage.complete
                    && u.producer_evidence
                        .iter()
                        .any(|e| e.source == "cargo-folded-v2")
            });
            let changes = if valid && root.starts_with(&ctx.root) && !ctx.force_full {
                changed.as_deref().map(|v| v.as_slice())
            } else {
                None
            };
            if changes.is_some_and(|changes| {
                changes
                    .iter()
                    .all(|p| !p.starts_with(&root) && !root.starts_with(p))
            }) {
                nested.extend(old);
                continue;
            }
            nested.extend(crate::cargo_artifacts::folded_units(
                &root,
                &draft.projects,
                &draft.dirs,
            )?);
        }
        draft.nested_artifacts = Arc::new(nested);
        Ok(vec![Event::CargoAnnotated(Arc::new(draft))])
    }
}
