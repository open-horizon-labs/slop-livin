//! Same-root warm-cache comparison; no cleanup or persistent user-store writes.
use std::{collections::HashMap, path::PathBuf, time::Instant};
fn main() -> anyhow::Result<()> {
    let root = std::fs::canonicalize(PathBuf::from(std::env::args_os().nth(1).expect("root")))?;
    let repeats = std::env::args()
        .nth(2)
        .unwrap_or("3".into())
        .parse::<usize>()?;
    let store = tempfile::tempdir()?;
    println!(
        "profile={} repeats={repeats} cache=warm/order-rotated",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    let baseline = swamp_core::walk::attribute_parallel(&root, &[(&root, "bench")], 1, u64::MAX);
    for round in 0..repeats {
        for slot in 0..3 {
            let mode = (round + slot) % 3;
            let start = Instant::now();
            let (attribution, count, size) = if mode == 0 {
                (
                    swamp_core::walk::attribute_parallel(&root, &[(&root, "bench")], 1, u64::MAX),
                    0,
                    0,
                )
            } else {
                let (send, recv) = swamp_core::folded::channel(8);
                std::thread::scope(|s| -> anyhow::Result<_> {
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
                    let path = store.path().join("measurements.parquet");
                    let count = if mode == 1 {
                        let mut n = 0;
                        for entry in recv {
                            entry.map_err(anyhow::Error::msg)?;
                            n += 1;
                        }
                        n
                    } else {
                        swamp_core::folded::write_measurements(&path, recv)?
                    };
                    Ok((
                        worker.join().unwrap(),
                        count,
                        if mode == 2 {
                            std::fs::metadata(path)?.len()
                        } else {
                            0
                        },
                    ))
                })?
            };
            anyhow::ensure!(
                attribution.walked_total == baseline.walked_total,
                "fixture changed during benchmark"
            );
            println!(
                "round={round} mode={} elapsed_ms={} entries={count} parquet_bytes={size}",
                ["walk", "collect", "persist"][mode],
                start.elapsed().as_millis()
            );
        }
    }
    Ok(())
}
