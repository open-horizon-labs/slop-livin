//! Replay-informed Cargo annotation. Failed coverage never reaches history.
use crate::bus::{Consumer, Ctx, Draft, Event, EventKind};
use anyhow::{Result, bail};
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
        for (root, workspace) in roots {
            let old = grouped
                .remove(&crate::artifact::NestedArtifact::storage_id(&root, ""))
                .unwrap_or_default();
            let valid = old.iter().any(|u| {
                u.path == root
                    && u.id == crate::artifact::NestedArtifact::storage_id(&root, "")
                    && u.coverage.complete
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
            let inspection = crate::cargo_artifacts::inspect_target_incremental(
                &root,
                Some(&workspace),
                &old,
                changes,
            );
            if !inspection.coverage.complete {
                bail!(
                    "Cargo observation incomplete at {}: {}; previous history retained",
                    root.display(),
                    inspection.coverage.limits.join("; ")
                );
            }
            nested.extend(inspection.units);
        }
        crate::cargo_artifacts::charge_physical(&mut nested);
        draft.nested_artifacts = Arc::new(nested);
        Ok(vec![Event::CargoAnnotated(Arc::new(draft))])
    }
}
