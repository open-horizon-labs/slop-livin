//! `ProjectsGrouped` → `SignalsComputed`: git activity per worktree (last
//! commit age, dirty, unpushed, locked, idle) plus the current branch,
//! computed in parallel at walk time.

use crate::bus::{Consumer, Ctx, Event, EventKind, WorktreeSignals};
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub struct SignalsConsumer;

#[async_trait::async_trait(?Send)]
impl Consumer for SignalsConsumer {
    fn name(&self) -> &str {
        "signals"
    }
    fn subscribes_to(&self) -> &[EventKind] {
        &[EventKind::ProjectsGrouped]
    }
    async fn on_event(&self, event: &Event, ctx: &Ctx<'_>) -> Result<Vec<Event>> {
        let Event::ProjectsGrouped {
            projects, rewalked, ..
        } = event
        else {
            return Ok(vec![]);
        };
        let all: Vec<(String, PathBuf)> = projects
            .iter()
            .flat_map(|p| {
                p.worktrees
                    .iter()
                    .map(|w| (w.worktree_id.clone(), w.path.clone()))
            })
            .collect();
        let mut by_worktree: HashMap<String, WorktreeSignals> = HashMap::new();
        // Incremental walk: a worktree FSEvents reported nothing under has
        // the same git state it had last time; age its signals instead of
        // running git on it again. Only the re-walked ones are recomputed.
        let mut todo: Vec<(String, PathBuf)> = all.clone();
        if let (Some(rewalked), Some(store)) = (rewalked, &ctx.store_dir)
            && let Some(prev) = crate::report::load_last_report(store, &ctx.root)
        {
            let elapsed = ctx.observed_at.saturating_sub(prev.observed_at);
            let rewalked: std::collections::HashSet<&String> = rewalked.iter().collect();
            let prev_rows: HashMap<&str, &crate::report::WorktreeRow> = prev
                .projects
                .iter()
                .flat_map(|p| p.worktrees.iter())
                .map(|w| (w.worktree_id.as_str(), w))
                .collect();
            todo.retain(|(id, path)| {
                if rewalked.contains(id) {
                    return true;
                }
                let Some(w) = prev_rows.get(id.as_str()) else {
                    return true;
                };
                let raw = crate::signals::RawSignals {
                    last_commit_age_secs: None,
                    dirty: w
                        .signals
                        .iter()
                        .find(|s| s.name == "dirty")
                        .map(|s| s.value == "dirty"),
                    unpushed: w
                        .signals
                        .iter()
                        .find(|s| s.name == "unpushed")
                        .and_then(|s| s.value.split(' ').next()?.parse().ok()),
                    locked: w
                        .signals
                        .iter()
                        .find(|s| s.name == "locked")
                        .map(|s| s.value == "locked"),
                    idle_for_secs: w.idle_secs,
                };
                // The gate appends `merge_complete` and `pull_request`
                // from the GitHub facts every run. Carrying a previous
                // report's rows forward wholesale re-adds them, and the
                // worktree line printed each one twice.
                let previous: Vec<crate::report::Signal> = w
                    .signals
                    .iter()
                    .filter(|s| s.name != "merge_complete" && s.name != "pull_request")
                    .cloned()
                    .collect();
                let (rows, raw) = crate::signals::age_signals(&previous, &raw, elapsed);
                by_worktree.insert(
                    id.clone(),
                    WorktreeSignals {
                        branch: w.branch.clone(),
                        path: path.clone(),
                        rows,
                        raw,
                    },
                );
                false
            });
        }
        let paths: Vec<PathBuf> = todo.iter().map(|(_, p)| p.clone()).collect();
        let computed = crate::signals::compute_signals_raw_parallel(&paths, ctx.observed_at);
        let mut it = computed.into_iter();
        for (id, path) in todo {
            let (rows, raw) = it.next().unwrap_or_else(|| {
                (
                    Vec::new(),
                    crate::signals::RawSignals {
                        last_commit_age_secs: None,
                        dirty: None,
                        unpushed: None,
                        locked: None,
                        idle_for_secs: None,
                    },
                )
            });
            by_worktree.insert(
                id,
                WorktreeSignals {
                    branch: crate::github::current_branch(&path),
                    path,
                    rows,
                    raw,
                },
            );
        }
        Ok(vec![Event::SignalsComputed {
            by_worktree: Arc::new(by_worktree),
        }])
    }
}
