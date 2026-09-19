//! Read-only project benchmark with an isolated, automatically removed store.
//! The second observation uses the real platform replay (may refuse and walk).
use std::{
    path::{Path, PathBuf},
    time::Instant,
};
fn size(p: &Path) -> u64 {
    if p.is_file() {
        return p.metadata().map(|m| m.len()).unwrap_or(0);
    }
    std::fs::read_dir(p)
        .map(|it| it.flatten().map(|e| size(&e.path())).sum())
        .unwrap_or(0)
}
fn main() -> anyhow::Result<()> {
    let root = PathBuf::from(std::env::args_os().nth(1).expect("project root"));
    let store = tempfile::tempdir()?;
    for full in [true, false, false] {
        let start = Instant::now();
        let report = swamp_core::report::report_full_mode(
            &root,
            None,
            false,
            Some(store.path()),
            Some("1h"),
            true,
            false,
            false,
            full,
        )?;
        let tests = report
            .nested_artifacts
            .iter()
            .filter(|u| u.role == swamp_core::artifact::ArtifactRole::TestExecutable)
            .count();
        println!(
            "full={full} elapsed={:?} units={} test_executables={tests} store_bytes={}",
            start.elapsed(),
            report.nested_artifacts.len(),
            size(store.path())
        );
        for note in report.notes.iter().filter(|n| n.contains("fsevents")) {
            println!("{note}");
        }
    }
    Ok(())
}
