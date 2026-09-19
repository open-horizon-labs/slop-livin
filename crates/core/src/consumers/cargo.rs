//! `RowsAssembled` → `CargoAnnotated`: read-only Rust build-layout facts.

use crate::bus::{Consumer, Ctx, Draft, Event, EventKind};
use anyhow::Result;
use std::sync::Arc;

pub struct CargoConsumer;

#[async_trait::async_trait(?Send)]
impl Consumer for CargoConsumer {
    fn name(&self) -> &str {
        "cargo-artifacts"
    }

    fn subscribes_to(&self) -> &[EventKind] {
        &[EventKind::RowsAssembled]
    }

    async fn on_event(&self, event: &Event, ctx: &Ctx<'_>) -> Result<Vec<Event>> {
        let Event::RowsAssembled(draft) = event else {
            return Ok(vec![]);
        };
        let mut draft: Draft = (**draft).clone();
        let cached = ctx
            .store_dir
            .as_deref()
            .and_then(|dir| crate::report::load_last_report(dir, &ctx.root))
            .map(|r| r.nested_artifacts)
            .unwrap_or_default();
        let cache_matches = cargo_cache_matches(&draft.projects, &cached);
        draft.nested_artifacts = if cache_matches {
            cached
        } else {
            crate::cargo_artifacts::inspect_projects(&draft.projects)
        };
        Ok(vec![Event::CargoAnnotated(Arc::new(draft))])
    }
}

fn cargo_cache_matches(
    projects: &[crate::report::ProjectRow],
    cached: &[crate::artifact::NestedArtifact],
) -> bool {
    let targets: Vec<_> = projects
        .iter()
        .flat_map(|p| p.worktrees.iter())
        .flat_map(|w| w.artifacts.iter())
        .filter(|a| {
            a.kind == crate::report::ArtifactKind::BuildOutput
                && a.path.is_dir()
                && (a.path.file_name().and_then(|n| n.to_str()) == Some("target")
                    || cached.iter().any(|u| {
                        u.role == crate::artifact::ArtifactRole::Container && u.path == a.path
                    }))
        })
        .collect();
    if targets.is_empty() {
        return true;
    }
    targets.iter().all(|row| {
        cached
            .iter()
            .find(|u| u.role == crate::artifact::ArtifactRole::Container && u.path == row.path)
            .is_some_and(|u| u.bytes == row.bytes && u.present)
    })
}
