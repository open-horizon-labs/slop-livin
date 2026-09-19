//! `RowsAssembled` → `GrowthAnnotated`: the reverse-delta store. Persists
//! this observation (or reads the store read-only) and fills growth and
//! regrowth on every artifact, directory and file row; groups directory
//! and file rows per worktree when the run asked for them.

use crate::bus::{Consumer, Ctx, Draft, Event, EventKind};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;

pub struct GrowthConsumer;

#[async_trait::async_trait(?Send)]
impl Consumer for GrowthConsumer {
    fn name(&self) -> &str {
        "growth"
    }
    fn subscribes_to(&self) -> &[EventKind] {
        &[EventKind::RowsAssembled]
    }
    async fn on_event(&self, event: &Event, ctx: &Ctx<'_>) -> Result<Vec<Event>> {
        let Event::RowsAssembled(draft) = event else {
            return Ok(vec![]);
        };
        let mut d: Draft = (**draft).clone();
        // Roll child totals up the directory chain first, so the store
        // records (and measures growth on) aggregated directory sizes.
        let roots = crate::report::artifact_roots(&d.projects);
        crate::report::aggregate_dir_totals(&mut d.dirs, &roots);
        if let Some(dir) = &ctx.store_dir {
            let volume_id = std::fs::metadata(&ctx.root)
                .map(|m| std::os::unix::fs::MetadataExt::dev(&m))
                .unwrap_or(0);
            let config = crate::growth::load_config(dir);
            let since_secs = ctx
                .since_override
                .as_deref()
                .and_then(crate::growth::parse_duration_secs)
                .or_else(|| crate::growth::parse_duration_secs(&config.since))
                .unwrap_or(24 * 3600);
            let trace = std::env::var("SLOP_LIVIN_TRACE").is_ok_and(|v| v != "0" && !v.is_empty());
            let t = std::time::Instant::now();
            if ctx.observe {
                crate::growth::observe_and_annotate(
                    dir,
                    volume_id,
                    &mut d.projects,
                    ctx.observed_at,
                    config.retention_days,
                    since_secs,
                )?;
                if trace {
                    eprintln!("[trace] growth: artifacts store: {:?}", t.elapsed());
                }
                let t = std::time::Instant::now();
                crate::growth::observe_and_annotate_dirs(
                    dir,
                    volume_id,
                    &mut d.dirs,
                    ctx.observed_at,
                    config.retention_days,
                    since_secs,
                )?;
                if trace {
                    eprintln!(
                        "[trace] growth: dirs store ({} rows): {:?}",
                        d.dirs.len(),
                        t.elapsed()
                    );
                }
                let t = std::time::Instant::now();
                crate::growth::observe_and_annotate_files(
                    dir,
                    volume_id,
                    &mut d.files,
                    ctx.observed_at,
                    config.retention_days,
                    since_secs,
                )?;
                if trace {
                    eprintln!(
                        "[trace] growth: files store ({} rows): {:?}",
                        d.files.len(),
                        t.elapsed()
                    );
                }
            } else {
                crate::growth::annotate_readonly(
                    dir,
                    volume_id,
                    &mut d.projects,
                    ctx.observed_at,
                    config.retention_days,
                    since_secs,
                )?;
                crate::growth::annotate_readonly_dirs(
                    dir,
                    volume_id,
                    &mut d.dirs,
                    ctx.observed_at,
                    config.retention_days,
                    since_secs,
                )?;
                crate::growth::annotate_readonly_files(
                    dir,
                    volume_id,
                    &mut d.files,
                    ctx.observed_at,
                    config.retention_days,
                    since_secs,
                )?;
            }
            d.schedule_line = Some(crate::schedule::header_line(
                dir,
                &ctx.root,
                ctx.observed_at,
            ));
        }
        // Interior rows of folded artifacts live in the store only: the
        // report shows an artifact as one unit.
        d.dirs
            .retain(|row| !crate::report::dir_inside_artifact(row, &roots));
        if ctx.include_dirs {
            let mut by_dir: HashMap<String, Vec<_>> = HashMap::new();
            for row in std::mem::take(&mut d.dirs) {
                by_dir.entry(row.worktree_id.clone()).or_default().push(row);
            }
            let mut by_file: HashMap<String, Vec<_>> = HashMap::new();
            for row in std::mem::take(&mut d.files) {
                by_file
                    .entry(row.worktree_id.clone())
                    .or_default()
                    .push(row);
            }
            d.dirs_by_worktree = Some(by_dir);
            d.files_by_worktree = Some(by_file);
        }
        Ok(vec![Event::GrowthAnnotated(Arc::new(d))])
    }
}
