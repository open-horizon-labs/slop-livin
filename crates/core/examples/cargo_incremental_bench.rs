//! Read-only incremental-pipeline benchmark. Injects notifications, not actual
//! filesystem changes; excludes native FSEvents delivery latency. Isolated store.
use std::{path::PathBuf, time::Instant};
use swamp_core::fs_events::{FsEventsPlan, FsEventsRequest, FsEventsSource};

struct Live(Vec<PathBuf>);
impl FsEventsSource for Live {
    fn replay(&self, _: &FsEventsRequest) -> FsEventsPlan {
        FsEventsPlan::from_live(self.0.clone(), 1000, None)
    }
}
fn main() -> anyhow::Result<()> {
    let root = std::fs::canonicalize(PathBuf::from(
        std::env::args_os().nth(1).expect("project root"),
    ))?;
    let store = tempfile::tempdir()?;
    swamp_core::report::report_full_mode(
        &root,
        None,
        false,
        Some(store.path()),
        Some("1h"),
        true,
        false,
        false,
        true,
    )?;
    let mut workloads = vec![
        ("no_changes", vec![]),
        ("source_directory", vec![root.join("crates/core/src")]),
        ("cargo_deps_directory", vec![root.join("target/debug/deps")]),
    ];
    if let Ok(entries) = std::fs::read_dir(root.join("target/debug/incremental")) {
        let mut groups: Vec<_> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .collect();
        groups.sort();
        if let Some(group) = groups.first() {
            workloads.push(("cargo_small_group", vec![group.clone()]));
        }
    }
    for (name, changed) in workloads {
        for _ in 0..3 {
            let start = Instant::now();
            let report = swamp_core::report::report_full_mode_with_source(
                &root,
                None,
                false,
                Some(store.path()),
                Some("1h"),
                true,
                false,
                false,
                false,
                &Live(changed.clone()),
            )?;
            let stale = report
                .projects
                .iter()
                .flat_map(|p| &p.worktrees)
                .flat_map(|w| &w.artifacts)
                .filter(|a| a.dedup_stale)
                .count();
            println!(
                "{name} elapsed={:?} units={} stale_unique_totals={stale}",
                start.elapsed(),
                report.nested_artifacts.len()
            );
            for n in report.notes.iter().filter(|n| n.contains("fsevents")) {
                println!("{n}");
            }
        }
    }
    Ok(())
}
