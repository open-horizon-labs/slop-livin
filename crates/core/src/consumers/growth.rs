//! `CargoAnnotated` → `GrowthAnnotated`: the reverse-delta store. Persists
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
        &[EventKind::CargoAnnotated]
    }
    async fn on_event(&self, event: &Event, ctx: &Ctx<'_>) -> Result<Vec<Event>> {
        let Event::CargoAnnotated(draft) = event else {
            return Ok(vec![]);
        };
        let mut d: Draft = (**draft).clone();
        // CargoAnnotated carries directory totals already aggregated once.
        let roots = crate::report::artifact_roots(&d.projects);
        let nested_shadow_paths =
            add_nested_history_rows(&mut d.projects, &d.nested_artifacts, ctx.observed_at);
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
            let trace = std::env::var("SWAMP_TRACE").is_ok_and(|v| v != "0" && !v.is_empty());
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
        let measured: HashMap<_, _> = d
            .dirs
            .iter()
            .map(|row| ((row.worktree_id.as_str(), row.rel_path.as_str()), row))
            .collect();
        for project in &mut d.projects {
            for wt in &mut project.worktrees {
                for artifact in &mut wt.artifacts {
                    let rel = artifact
                        .path
                        .strip_prefix(&wt.path)
                        .unwrap_or(&artifact.path)
                        .to_string_lossy();
                    if !roots.contains(&(wt.worktree_id.clone(), rel.to_string())) {
                        continue;
                    }
                    if let Some(dir) = measured.get(&(wt.worktree_id.as_str(), rel.as_ref())) {
                        artifact.allocated_bytes = Some(dir.allocated_total);
                        artifact.allocated_growth_bytes = dir.growth_bytes;
                    }
                }
            }
        }
        // Interior rows of folded artifacts live in the store only: the
        // report shows an artifact as one unit.
        d.dirs
            .retain(|row| !crate::report::dir_inside_artifact(row, &roots));
        copy_nested_history(
            &mut d.projects,
            Arc::make_mut(&mut d.nested_artifacts).as_mut_slice(),
            &nested_shadow_paths,
        );
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

fn add_nested_history_rows(
    projects: &mut [crate::report::ProjectRow],
    nested: &[crate::artifact::NestedArtifact],
    observed_at: u64,
) -> Vec<(String, std::path::PathBuf)> {
    let mut paths = Vec::new();
    for unit in nested {
        let Some((_, wt)) = projects
            .iter_mut()
            .flat_map(|p| p.worktrees.iter_mut())
            .filter(|wt| {
                unit.path.starts_with(&wt.path)
                    || wt
                        .artifacts
                        .iter()
                        .any(|a| a.source.tool != "cargo.layout" && unit.path.starts_with(&a.path))
            })
            .max_by_key(|wt| wt.path.components().count())
            .map(|wt| ((), wt))
        else {
            continue;
        };
        wt.artifacts.push(crate::report::ArtifactRow {
            kind: crate::report::ArtifactKind::Unknown,
            path: unit.path.clone(),
            bytes: unit.bytes,
            mtime_max: unit.mtime_max,
            ecosystem: None,
            hardlinked: matches!(unit.membership, crate::artifact::Membership::SharedHardlink),
            dedup_stale: false,
            allocated_bytes: None,
            allocated_growth_bytes: None,
            local_bytes: unit.physical_bytes,
            track: None,
            growth_bytes: unit.growth_bytes,
            regrowth_count: unit.regrowth_count,
            observed_at,
            confidence: crate::entities::Confidence::Medium,
            source: crate::report::Source::new("cargo.layout"),
            note: Some(format!("nested-id={}", unit.id)),
            created_at: None,
            containers: Vec::new(),
            shared_with: Vec::new(),
            dangling: false,
        });
        paths.push((unit.id.clone(), unit.path.clone()));
    }
    paths
}

fn copy_nested_history(
    projects: &mut [crate::report::ProjectRow],
    nested: &mut [crate::artifact::NestedArtifact],
    paths: &[(String, std::path::PathBuf)],
) {
    let _ = paths;
    let facts: HashMap<_, _> = projects
        .iter()
        .flat_map(|p| &p.worktrees)
        .flat_map(|w| &w.artifacts)
        .filter(|a| a.source.tool == "cargo.layout")
        .map(|a| (a.path.clone(), (a.growth_bytes, a.regrowth_count)))
        .collect();
    for unit in nested {
        if let Some((growth, regrowth)) = facts.get(&unit.path) {
            unit.growth_bytes = *growth;
            unit.regrowth_count = *regrowth;
        }
    }
    for project in projects {
        for wt in &mut project.worktrees {
            wt.artifacts.retain(|a| a.source.tool != "cargo.layout");
        }
    }
}
