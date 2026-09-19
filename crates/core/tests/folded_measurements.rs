use anyhow::Result;
use std::{collections::HashMap, fs, os::unix::fs::symlink, sync::mpsc::sync_channel};
use swamp_core::{folded, walk};

#[test]
fn existing_folded_walk_emits_once_and_carry_emits_nothing() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let root = fs::canonicalize(tmp.path())?;
    let target = root.join("target");
    fs::create_dir_all(target.join("debug/deps"))?;
    fs::write(target.join("debug/deps/a"), vec![1; 8192])?;
    fs::hard_link(target.join("debug/deps/a"), target.join("debug/deps/b"))?;
    let odd = target.join("odd");
    fs::write(&odd, b"odd")?;
    symlink(root.join("outside"), target.join("link"))?;
    let (send, recv) = sync_channel(2);
    let (attribution, entries) = std::thread::scope(|s| {
        let worker = s.spawn(|| {
            walk::attribute_parallel_recording(&root, &[(&root, "w")], 1, 0, HashMap::new(), send)
        });
        let entries = recv.into_iter().collect::<Vec<_>>();
        (worker.join().unwrap(), entries)
    });
    let entries = entries
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(anyhow::Error::msg)?;
    assert_eq!(entries.len(), 7); // target/debug/deps, two links, odd, symlink
    let mut unique = std::collections::HashSet::new();
    assert!(entries.iter().all(|e| unique.insert(e.relative.clone())));
    assert!(entries.iter().any(|e| e.relative == b"odd"));
    assert_eq!(entries.iter().filter(|e| e.kind == 2).count(), 1);
    let a = entries
        .iter()
        .find(|e| e.relative == b"debug/deps/a")
        .unwrap();
    let b = entries
        .iter()
        .find(|e| e.relative == b"debug/deps/b")
        .unwrap();
    assert_eq!((a.device, a.inode), (b.device, b.inode));
    let artifact = attribution.artifacts_by_worktree["w"]
        .iter()
        .find(|a| a.path == target)
        .unwrap()
        .clone();
    let (send, recv) = sync_channel(2);
    let carried = walk::attribute_parallel_recording(
        &root,
        &[(&root, "w")],
        2,
        0,
        HashMap::from([(target, artifact.clone())]),
        send,
    );
    assert_eq!(
        recv.into_iter().count(),
        0,
        "unchanged roots must not emit descendants"
    );
    assert_eq!(
        carried.artifacts_by_worktree["w"]
            .iter()
            .find(|a| a.path == artifact.path)
            .unwrap()
            .bytes,
        artifact.bytes
    );
    Ok(())
}

#[test]
fn compact_batches_use_existing_parquet_writer_and_failed_scan_is_not_published() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let file = tmp.path().join("current.parquet");
    let entry = folded::Entry {
        container: b"target".to_vec(),
        relative: b"debug/\xff".to_vec(),
        device: 1,
        inode: 2,
        links: 1,
        logical: 8192,
        allocated: 8192,
        modified_ns: 42,
        changed_ns: 42,
        kind: 0,
    };
    let count = folded::BATCH_ROWS * 3 + 1;
    assert_eq!(
        folded::write_measurements(&file, (0..count).map(|_| Ok(entry.clone())))?,
        count
    );
    let reader = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(
        fs::File::open(&file)?,
    )?;
    assert_eq!(reader.metadata().num_row_groups(), 4);
    assert_eq!(reader.metadata().file_metadata().num_rows(), count as i64);
    let previous = fs::read(&file)?;
    assert!(folded::write_measurements(&file, [Ok(entry), Err("lost access".into())]).is_err());
    assert_eq!(fs::read(&file)?, previous);
    Ok(())
}
