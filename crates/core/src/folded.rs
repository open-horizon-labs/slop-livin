//! Compact facts emitted by the existing folded-folder measurement pass.
//! These are measurements, not rich report rows, ownership or cleanup verdicts.
use anyhow::Result;
use arrow_array::{ArrayRef, BinaryArray, Int64Array, RecordBatch, UInt8Array, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use std::{
    fs::Metadata,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::Path,
    sync::Arc,
};

pub type EntryResult = std::result::Result<Entry, String>;
pub const BATCH_ROWS: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub container: Vec<u8>,
    pub relative: Vec<u8>,
    pub device: u64,
    pub inode: u64,
    pub links: u64,
    pub logical: u64,
    pub allocated: u64,
    pub modified_ns: i64,
    pub changed_ns: i64,
    /// 0 regular, 1 directory, 2 symlink, 3 other.
    pub kind: u8,
}

impl Entry {
    pub fn measured(root: &Path, path: &Path, m: &Metadata) -> Self {
        Self {
            container: root.as_os_str().as_bytes().into(),
            relative: path
                .strip_prefix(root)
                .expect("folded entry inside root")
                .as_os_str()
                .as_bytes()
                .into(),
            device: m.dev(),
            inode: m.ino(),
            links: m.nlink(),
            logical: if m.is_file() { m.len() } else { 0 },
            allocated: if m.is_file() { m.blocks() * 512 } else { 0 },
            modified_ns: m
                .mtime()
                .saturating_mul(1_000_000_000)
                .saturating_add(m.mtime_nsec()),
            changed_ns: m
                .ctime()
                .saturating_mul(1_000_000_000)
                .saturating_add(m.ctime_nsec()),
            kind: if m.is_file() {
                0
            } else if m.is_dir() {
                1
            } else if m.file_type().is_symlink() {
                2
            } else {
                3
            },
        }
    }
}

fn schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("container", DataType::Binary, false),
        Field::new("relative", DataType::Binary, false),
        Field::new("device", DataType::UInt64, false),
        Field::new("inode", DataType::UInt64, false),
        Field::new("links", DataType::UInt64, false),
        Field::new("logical", DataType::UInt64, false),
        Field::new("allocated", DataType::UInt64, false),
        Field::new("modified_ns", DataType::Int64, false),
        Field::new("changed_ns", DataType::Int64, false),
        Field::new("kind", DataType::UInt8, false),
    ]))
}

fn batch(entries: &[Entry]) -> Result<RecordBatch> {
    let mut columns: Vec<ArrayRef> = vec![
        Arc::new(BinaryArray::from_iter_values(
            entries.iter().map(|e| e.container.as_slice()),
        )),
        Arc::new(BinaryArray::from_iter_values(
            entries.iter().map(|e| e.relative.as_slice()),
        )),
    ];
    for accessor in [
        |e: &Entry| e.device,
        |e: &Entry| e.inode,
        |e: &Entry| e.links,
        |e: &Entry| e.logical,
        |e: &Entry| e.allocated,
    ] {
        columns.push(Arc::new(UInt64Array::from_iter_values(
            entries.iter().map(accessor),
        )));
    }
    columns.push(Arc::new(Int64Array::from_iter_values(
        entries.iter().map(|e| e.modified_ns),
    )));
    columns.push(Arc::new(Int64Array::from_iter_values(
        entries.iter().map(|e| e.changed_ns),
    )));
    columns.push(Arc::new(UInt8Array::from_iter_values(
        entries.iter().map(|e| e.kind),
    )));
    Ok(RecordBatch::try_new(schema(), columns)?)
}

/// A bounded receiver can feed this directly while the existing walker runs.
/// Errors prevent publication; no partial scan becomes a valid observation.
/// This is a measurement segment, not a second authoritative history store.
pub fn write_measurements(
    path: &Path,
    entries: impl IntoIterator<Item = EntryResult>,
) -> Result<usize> {
    let mut entries = entries.into_iter();
    let mut count = 0;
    let batches = std::iter::from_fn(|| {
        let mut rows = Vec::with_capacity(BATCH_ROWS);
        for _ in 0..BATCH_ROWS {
            match entries.next() {
                Some(Ok(entry)) => rows.push(entry),
                Some(Err(e)) => return Some(Err(anyhow::anyhow!(e))),
                None => break,
            }
        }
        if rows.is_empty() {
            None
        } else {
            count += rows.len();
            Some(batch(&rows))
        }
    });
    crate::growth::write_parquet_batches_atomic(path, schema(), batches, 3)?;
    Ok(count)
}
