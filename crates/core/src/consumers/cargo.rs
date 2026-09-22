//! Build-artifact identification over the folded directory
//! measurements, through the static adapter registry.
//!
//! The consumer used to be Cargo's and only Cargo's: it called
//! `cargo_artifacts::folded_units` by name. Adding Node would have meant
//! a second named call, and the fourth would have meant a `match`. It
//! now builds one [`crate::build_adapters::BuildCtx`] and hands it to
//! [`crate::build_adapters::identify_all`], so a new ecosystem is one
//! line in `build_adapters::Registry::with_builtins()` and nothing here
//! changes (`.oh/guardrails/build-adapters-are-pluggable.md`).
//!
//! The consumer keeps the two jobs an adapter must not do: deciding
//! *which* containers exist in the authorized scope, and deciding
//! whether the previous pass's units may be replayed. The replay gate is
//! [`crate::fs_events::EventCoverage`], built from this root's trusted
//! window -- the same gate stack/13 put on the agent containers, and
//! never a directory stamp.
use crate::build_adapters::{BuildContainer, BuildCtx, ContainerCache, FoldedDir, FoldedIndex};
use crate::bus::{Consumer, Ctx, Draft, Event, EventKind};
use anyhow::Result;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub struct CargoConsumer {
    changed: Mutex<Option<Arc<Vec<PathBuf>>>>,
    /// The static adapter set, built once when the consumer is
    /// constructed rather than per event. Nothing registers an adapter
    /// at runtime; `Registry::with_builtins()` is a fixed list, and the
    /// `build_adapters_are_pluggable` audit checks it against the module
    /// set so an unregistered adapter fails the build.
    adapters: crate::build_adapters::registry::Registry,
}

impl Default for CargoConsumer {
    fn default() -> Self {
        Self {
            changed: Mutex::new(None),
            adapters: crate::build_adapters::registry::Registry::with_builtins(),
        }
    }
}

#[async_trait::async_trait(?Send)]
impl Consumer for CargoConsumer {
    fn name(&self) -> &str {
        "build-artifacts"
    }
    fn subscribes_to(&self) -> &[EventKind] {
        &[EventKind::RootObserved, EventKind::RowsAssembled]
    }
    async fn on_event(
        &self,
        event: &Event,
        ctx: &Ctx<'_>,
        _stage: &crate::bus::Stage,
    ) -> Result<Vec<Event>> {
        if let Event::RootObserved { changed_paths, .. } = event {
            *self.changed.lock().unwrap() = changed_paths.clone();
            return Ok(vec![]);
        }
        let Event::RowsAssembled(draft) = event else {
            return Ok(vec![]);
        };
        let mut draft: Draft = (**draft).clone();
        // Aggregate once, before either identification or history uses it.
        let artifact_roots = crate::report::artifact_roots(&draft.projects);
        crate::report::aggregate_dir_totals(&mut draft.dirs, &artifact_roots);

        let previous = ctx
            .store_dir
            .as_deref()
            .and_then(|d| crate::report::load_last_report(d, &ctx.root))
            .map(|r| (r.observed_at, r.nested_artifacts));

        // The replay gate. A root this pass replayed successfully leaves
        // a trusted window here; a full walk, a refusal or a store-less
        // call leaves none, and then nothing is reused.
        let mut coverage = crate::fs_events::EventCoverage::untrusted();
        if !ctx.force_full
            && let Some((changed, since)) = ctx.event_window.lock().unwrap().clone()
        {
            coverage.trust(ctx.root.clone(), changed, since);
        }
        let cache = match &previous {
            Some((observed_at, units)) if !ctx.force_full => {
                ContainerCache::from_previous(units.clone(), *observed_at)
            }
            _ => ContainerCache::disabled(),
        };

        let folded = folded_index(&draft);
        let build_ctx = BuildCtx::new(ctx.observed_at, &folded, &coverage, &cache);
        let projects = project_candidates(&draft);
        let shared = shared_containers(ctx);
        let nested =
            crate::build_adapters::identify_all(&self.adapters, &projects, &shared, &build_ctx);

        draft.nested_artifacts = Arc::new(nested);
        Ok(vec![Event::CargoAnnotated(Arc::new(draft))])
    }
}

/// The folded walk's directory rows, as absolute paths.
///
/// This is the whole structural input the adapters get, and building it
/// once per pass is what keeps "no second traversal" true: every adapter
/// reads from this map, and none of them lists a directory the walk
/// already listed.
fn folded_index(draft: &Draft) -> FoldedIndex {
    let worktrees: std::collections::HashMap<&str, &std::path::Path> = draft
        .projects
        .iter()
        .flat_map(|p| &p.worktrees)
        .map(|w| (w.worktree_id.as_str(), w.path.as_path()))
        .collect();
    FoldedIndex::from_dirs(draft.dirs.iter().filter_map(|d| {
        let worktree = worktrees.get(d.worktree_id.as_str())?;
        Some(FoldedDir {
            path: worktree.join(&d.rel_path),
            allocated_total: d.allocated_total,
            // `mod_time_min` is minutes since the epoch (the walk's own
            // resolution). Seconds here, so an adapter never has to know
            // which unit a stored column is in.
            mtime_max: d.mod_time_min.max(0) as u64 * 60,
            complete: d.complete,
        })
    }))
}

/// Each worktree, with the artifact directories the walk classified
/// beneath it.
///
/// Adapters decide which of these are theirs, by name and by the marker
/// files at the checkout root. Nothing here knows what a `target/` or a
/// `node_modules` is -- that knowledge is the adapter's, which is the
/// point.
fn project_candidates(draft: &Draft) -> Vec<(PathBuf, Vec<PathBuf>)> {
    let mut out = Vec::new();
    for project in &draft.projects {
        for wt in &project.worktrees {
            let candidates: Vec<PathBuf> = wt
                .artifacts
                .iter()
                .filter(|a| !a.kind.is_worktree_remainder() && a.path.is_dir())
                .map(|a| a.path.clone())
                .collect();
            out.push((wt.path.clone(), candidates));
        }
    }
    out
}

/// Stores shared across every project: an npm/pnpm cache, a Gradle user
/// home, a Maven local repository.
///
/// Empty in this chunk, deliberately. These are detector-resolved
/// locations (`crate::locations`, #47) rather than artifact rows beneath
/// a checkout, and `external::discover_and_measure` already owns their
/// measurement and their history window. Joining them in from here would
/// be a second observation of the same bytes in the same pass, which is
/// the shape `.oh/guardrails/history-sweeps-are-owned.md` exists to
/// stop, and it would need an ownership decision this chunk has no
/// mandate to make.
///
/// The adapters' shared-store identification is written and tested
/// (`BuildContainer::shared_store`, and the store tests in
/// `build_adapters::{node,gradle,maven}`); what is missing is the join.
/// Saying so here is more honest than wiring a second traversal to make
/// a column non-empty.
fn shared_containers(_ctx: &Ctx<'_>) -> Vec<BuildContainer> {
    Vec::new()
}
