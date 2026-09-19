//! Read-only measurement of the existing folded walk with compact entry output.
//! Uses a disposable store outside the scanned root; does not clean artifacts.
use std::{collections::HashMap, path::PathBuf, sync::mpsc::sync_channel, time::Instant};
fn main() -> anyhow::Result<()> {
    let root = std::fs::canonicalize(PathBuf::from(std::env::args_os().nth(1).expect("root")))?;
    let store = tempfile::tempdir()?;
    let path = store.path().join("measurements.parquet");
    let (send, recv) = sync_channel(swamp_core::folded::BATCH_ROWS);
    let started = Instant::now();
    let (attribution, count) = std::thread::scope(|s| -> anyhow::Result<_> {
        let worker = s.spawn(|| {
            swamp_core::walk::attribute_parallel_recording(
                &root,
                &[(&root, "bench")],
                1,
                u64::MAX,
                HashMap::new(),
                send,
            )
        });
        let count = swamp_core::folded::write_measurements(&path, recv)?;
        Ok((worker.join().unwrap(), count))
    })?;
    println!(
        "initial elapsed={:?} entries={count} parquet_bytes={}",
        started.elapsed(),
        std::fs::metadata(&path)?.len()
    );
    let carry = attribution.artifacts_by_worktree["bench"]
        .iter()
        .filter(|a| !a.kind.is_worktree_remainder())
        .map(|a| (a.path.clone(), a.clone()))
        .collect();
    let (send, recv) = sync_channel(swamp_core::folded::BATCH_ROWS);
    let started = Instant::now();
    let count = std::thread::scope(|s| {
        let worker = s.spawn(|| {
            swamp_core::walk::attribute_parallel_recording(
                &root,
                &[(&root, "bench")],
                2,
                u64::MAX,
                carry,
                send,
            )
        });
        let count = recv.into_iter().count();
        worker.join().unwrap();
        count
    });
    anyhow::ensure!(count == 0, "unchanged folded interiors were visited");
    println!(
        "carried elapsed={:?} interior_entries={count} detail_bytes_written=0",
        started.elapsed()
    );
    Ok(())
}
