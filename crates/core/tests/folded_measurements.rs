use anyhow::Result;
use std::{collections::HashMap, fs, os::unix::fs::symlink, sync::Arc};
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
    let (send, recv) = swamp_core::folded::channel(2);
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
    assert!(
        Arc::ptr_eq(&a.container, &b.container),
        "same-directory entries share container storage"
    );
    let artifact = attribution.artifacts_by_worktree["w"]
        .iter()
        .find(|a| a.path == target)
        .unwrap()
        .clone();
    let (send, recv) = swamp_core::folded::channel(2);
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
fn batched_transfer_preserves_full_batches_tail_and_receiver_disconnect() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let root = fs::canonicalize(tmp.path())?;
    let target = root.join("target");
    fs::create_dir(&target)?;
    for i in 0..600 {
        fs::write(target.join(format!("{i}")), b"x")?;
    }
    let (send, recv) = folded::channel(1);
    let entries = std::thread::scope(|s| {
        let worker = s.spawn(|| {
            walk::attribute_parallel_recording(&root, &[(&root, "w")], 1, 0, HashMap::new(), send)
        });
        let entries = recv.collect::<Vec<_>>();
        worker.join().unwrap();
        entries
    });
    let entries = entries
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(anyhow::Error::msg)?;
    assert_eq!(entries.len(), 601);
    assert_eq!(
        entries
            .iter()
            .map(|e| &e.relative)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        601
    );
    let (send, recv) = folded::channel(1);
    drop(recv);
    // A failed writer must not leave worker threads blocked on a full queue.
    walk::attribute_parallel_recording(&root, &[(&root, "w")], 1, 0, HashMap::new(), send);
    Ok(())
}

#[test]
fn compact_batches_use_existing_parquet_writer_and_failed_scan_is_not_published() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let file = tmp.path().join("current.parquet");
    let entry = folded::Entry {
        container: Arc::from(b"target".as_slice()),
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
    let first = reader.build()?.next().unwrap()?;
    let names = first
        .column(1)
        .as_any()
        .downcast_ref::<arrow_array::BinaryArray>()
        .unwrap();
    assert_eq!(names.value(0), b"debug/\xff");
    let previous = fs::read(&file)?;
    assert!(
        folded::write_measurements(
            &file,
            (0..folded::BATCH_ROWS + 1)
                .map(|_| Ok(entry.clone()))
                .chain(std::iter::once(Err("lost access".into())))
        )
        .is_err()
    );
    assert_eq!(fs::read(&file)?, previous);
    assert_eq!(
        fs::read_dir(tmp.path())?.count(),
        1,
        "failed output leaves no partial file"
    );
    Ok(())
}
