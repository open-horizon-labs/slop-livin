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
        let Event::ProjectsGrouped { projects, .. } = event else {
            return Ok(vec![]);
        };
        let worktrees: Vec<(String, PathBuf)> = projects
            .iter()
            .flat_map(|p| {
                p.worktrees
                    .iter()
                    .map(|w| (w.worktree_id.clone(), w.path.clone()))
            })
            .collect();
        let paths: Vec<PathBuf> = worktrees.iter().map(|(_, p)| p.clone()).collect();
        let computed = crate::signals::compute_signals_raw_parallel(&paths, ctx.observed_at);
        let mut by_worktree: HashMap<String, WorktreeSignals> = HashMap::new();
        let mut it = computed.into_iter();
        for (id, path) in worktrees {
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
