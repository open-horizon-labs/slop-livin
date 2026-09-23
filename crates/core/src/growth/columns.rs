//! The history tables' rows and their Parquet columns: the only part of
//! `growth` that names Arrow (`arrow_array`/`arrow_schema`), and the only
//! place a stored artifact or external row can change.
//!
//! Two properties that used to be call-graph audits are types here:
//!
//! * **Every change to a known row appends a reverse delta**
//!   (`.oh/guardrails/reverse-delta-current-plus-deltas.md`). A
//!   [`StoredRow`]/[`StoredExternalRow`] has no public field and no
//!   setter: it changes only through [`ArtifactHistory::observe`] /
//!   [`ArtifactHistory::tombstone`] (and the external twins), which push
//!   the previous value onto the pending delta before changing it, and
//!   the table reaches disk only through `commit`, which writes the delta
//!   first. `write_rows` is private to this module, so rewriting
//!   `current.parquet` without its delta does not compile from `growth`.
//! * **Only an owned row is tombstoned; only a tombstoned row regrows**
//!   (`.oh/guardrails/history-sweeps-are-owned.md`,
//!   `coverage-changes-are-not-storage-changes.md`). `tombstone` takes an
//!   [`Owned`], which only an ownership's `claim` can produce, and
//!   `regrowth_count` moves only inside `observe`, when a tombstoned row
//!   is seen again. `row.present = false` and
//!   `row.regrowth_count += 1` in `growth.rs` are private-field errors
//!   (`crates/core/tests/compile_fail/`).

use anyhow::{Context, Result};
use arrow_array::{
    Array, ArrayRef, BooleanArray, Int32Array, Int64Array, RecordBatch, StringArray, UInt32Array,
    UInt64Array,
};
use arrow_schema::{DataType, Field, Schema};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// One stored row. Used both for `current.parquet` (where `bytes`/
/// `present` are the latest known value) and for delta files (where they
/// are the *previous* value, before the observation at `observed_at`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StoredRow {
    project_id: String,
    worktree_id: String,
    kind: String,
    rel_path: String,
    bytes: u64,
    local_bytes: u64,
    /// Newest file mtime inside the unit; 0 when unknown (older stores).
    mtime_max: u64,
    /// Whether the unit contains hardlinked files. Missing in a store
    /// written before this column existed, where it reads `true`: the
    /// conservative answer: unique totals need reconciliation after changes.
    hardlinked: bool,
    dedup_stale: bool,
    present: bool,
    observed_at: u64,
    regrowth_count: u32,
}

fn schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("project_id", DataType::Utf8, false),
        Field::new("worktree_id", DataType::Utf8, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new("rel_path", DataType::Utf8, false),
        Field::new("bytes", DataType::UInt64, false),
        Field::new("present", DataType::Boolean, false),
        Field::new("observed_at", DataType::UInt64, false),
        Field::new("regrowth_count", DataType::UInt32, false),
        Field::new("local_bytes", DataType::UInt64, false),
        Field::new("mtime_max", DataType::UInt64, false),
        Field::new("hardlinked", DataType::Boolean, false),
        Field::new("dedup_stale", DataType::Boolean, false),
    ]))
}

fn write_rows(path: &Path, rows: &[StoredRow]) -> Result<()> {
    let schema = schema();
    let project_ids: Vec<&str> = rows.iter().map(|r| r.project_id.as_str()).collect();
    let worktree_ids: Vec<&str> = rows.iter().map(|r| r.worktree_id.as_str()).collect();
    let kinds: Vec<&str> = rows.iter().map(|r| r.kind.as_str()).collect();
    let rel_paths: Vec<&str> = rows.iter().map(|r| r.rel_path.as_str()).collect();
    let bytes: Vec<u64> = rows.iter().map(|r| r.bytes).collect();
    let present: Vec<bool> = rows.iter().map(|r| r.present).collect();
    let observed_at: Vec<u64> = rows.iter().map(|r| r.observed_at).collect();
    let regrowth: Vec<u32> = rows.iter().map(|r| r.regrowth_count).collect();
    let local_bytes: Vec<u64> = rows.iter().map(|r| r.local_bytes).collect();
    let mtime_max: Vec<u64> = rows.iter().map(|r| r.mtime_max).collect();
    let hardlinked: Vec<bool> = rows.iter().map(|r| r.hardlinked).collect();

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(project_ids)) as ArrayRef,
            Arc::new(StringArray::from(worktree_ids)),
            Arc::new(StringArray::from(kinds)),
            Arc::new(StringArray::from(rel_paths)),
            Arc::new(UInt64Array::from(bytes)),
            Arc::new(BooleanArray::from(present)),
            Arc::new(UInt64Array::from(observed_at)),
            Arc::new(UInt32Array::from(regrowth)),
            Arc::new(UInt64Array::from(local_bytes)),
            Arc::new(UInt64Array::from(mtime_max)),
            Arc::new(BooleanArray::from(hardlinked)),
            Arc::new(BooleanArray::from(
                rows.iter().map(|r| r.dedup_stale).collect::<Vec<_>>(),
            )),
        ],
    )?;
    crate::fs_gate::columns::write_parquet_atomic(
        path,
        schema,
        std::iter::once(Ok(batch)),
        super::ARTIFACT_ZSTD_LEVEL,
    )
}

pub(super) fn read_rows(path: &Path) -> Result<Vec<StoredRow>> {
    let Some(reader) = crate::fs_gate::columns::open_parquet(path).with_context(|| {
        format!(
            "read {} (delete it to rebuild this store from a full walk)",
            path.display()
        )
    })?
    else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch?;
        let project_id = downcast_str(&batch, "project_id")?;
        let worktree_id = downcast_str(&batch, "worktree_id")?;
        let kind = downcast_str(&batch, "kind")?;
        let rel_path = downcast_str(&batch, "rel_path")?;
        let bytes = downcast_u64(&batch, "bytes")?;
        let present = downcast_bool(&batch, "present")?;
        let observed_at = downcast_u64(&batch, "observed_at")?;
        let regrowth = downcast_u32(&batch, "regrowth_count")?;
        // Stores written before #29 have no local_bytes column: fall back
        // to `bytes` so incremental deltas degrade to the old behavior.
        let local_bytes = downcast_u64(&batch, "local_bytes").ok();
        // Likewise stores written before artifact age was recorded.
        let mtime_max = downcast_u64(&batch, "mtime_max").ok();
        let hardlinked = downcast_bool(&batch, "hardlinked").ok();
        let dedup_stale = downcast_bool(&batch, "dedup_stale").ok();
        for i in 0..batch.num_rows() {
            rows.push(StoredRow {
                project_id: project_id.value(i).to_string(),
                worktree_id: worktree_id.value(i).to_string(),
                kind: kind.value(i).to_string(),
                rel_path: rel_path.value(i).to_string(),
                bytes: bytes.value(i),
                local_bytes: local_bytes
                    .as_ref()
                    .map(|c| c.value(i))
                    .unwrap_or_else(|| bytes.value(i)),
                mtime_max: mtime_max.as_ref().map(|c| c.value(i)).unwrap_or(0),
                hardlinked: hardlinked.as_ref().map(|c| c.value(i)).unwrap_or(true),
                dedup_stale: dedup_stale.as_ref().map(|c| c.value(i)).unwrap_or(false),
                present: present.value(i),
                observed_at: observed_at.value(i),
                regrowth_count: regrowth.value(i),
            });
        }
    }
    Ok(rows)
}

pub(super) fn downcast_str<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a StringArray> {
    batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<StringArray>())
        .with_context(|| format!("column {name} is not Utf8"))
}

pub(super) fn downcast_u64<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a UInt64Array> {
    batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<UInt64Array>())
        .with_context(|| format!("column {name} is not UInt64"))
}

pub(super) fn downcast_u32<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a UInt32Array> {
    batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<UInt32Array>())
        .with_context(|| format!("column {name} is not UInt32"))
}

pub(super) fn downcast_i64<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a Int64Array> {
    batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<Int64Array>())
        .with_context(|| format!("column {name} is not Int64"))
}

pub(super) fn downcast_bool<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a BooleanArray> {
    batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<BooleanArray>())
        .with_context(|| format!("column {name} is not Boolean"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StoredDirRow {
    pub(super) worktree_id: String,
    pub(super) rel_path: String,
    pub(super) parent_rel_path: Option<String>,
    pub(super) allocated_total: u64,
    pub(super) own_allocated: u64,
    pub(super) file_count: u32,
    pub(super) entry_count: u32,
    pub(super) symlink_count: u32,
    pub(super) mod_time_min: i32,
    pub(super) complete: bool,
    pub(super) observed_at: u64,
}

pub(super) fn dirs_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("worktree_id", DataType::Utf8, false),
        Field::new("rel_path", DataType::Utf8, false),
        Field::new("parent_rel_path", DataType::Utf8, true),
        Field::new("allocated_total", DataType::UInt64, false),
        Field::new("own_allocated", DataType::UInt64, false),
        Field::new("file_count", DataType::UInt32, false),
        Field::new("entry_count", DataType::UInt32, false),
        Field::new("symlink_count", DataType::UInt32, false),
        Field::new("mod_time_min", DataType::Int32, false),
        Field::new("complete", DataType::Boolean, false),
        Field::new("observed_at", DataType::UInt64, false),
    ]))
}

pub(super) fn write_dir_rows(path: &Path, rows: &[StoredDirRow], zstd_level: i32) -> Result<()> {
    let schema = dirs_schema();
    let worktree_ids: Vec<&str> = rows.iter().map(|r| r.worktree_id.as_str()).collect();
    let rel_paths: Vec<&str> = rows.iter().map(|r| r.rel_path.as_str()).collect();
    let parent_rel_paths: Vec<Option<&str>> =
        rows.iter().map(|r| r.parent_rel_path.as_deref()).collect();
    let allocated_total: Vec<u64> = rows.iter().map(|r| r.allocated_total).collect();
    let own_allocated: Vec<u64> = rows.iter().map(|r| r.own_allocated).collect();
    let file_count: Vec<u32> = rows.iter().map(|r| r.file_count).collect();
    let entry_count: Vec<u32> = rows.iter().map(|r| r.entry_count).collect();
    let symlink_count: Vec<u32> = rows.iter().map(|r| r.symlink_count).collect();
    let mod_time_min: Vec<i32> = rows.iter().map(|r| r.mod_time_min).collect();
    let complete: Vec<bool> = rows.iter().map(|r| r.complete).collect();
    let observed_at: Vec<u64> = rows.iter().map(|r| r.observed_at).collect();

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(worktree_ids)) as ArrayRef,
            Arc::new(StringArray::from(rel_paths)),
            Arc::new(StringArray::from(parent_rel_paths)),
            Arc::new(UInt64Array::from(allocated_total)),
            Arc::new(UInt64Array::from(own_allocated)),
            Arc::new(UInt32Array::from(file_count)),
            Arc::new(UInt32Array::from(entry_count)),
            Arc::new(UInt32Array::from(symlink_count)),
            Arc::new(Int32Array::from(mod_time_min)),
            Arc::new(BooleanArray::from(complete)),
            Arc::new(UInt64Array::from(observed_at)),
        ],
    )?;
    crate::fs_gate::columns::write_parquet_atomic(
        path,
        schema,
        std::iter::once(Ok(batch)),
        zstd_level,
    )
}

pub(super) fn read_dir_rows(path: &Path) -> Result<Vec<StoredDirRow>> {
    let Some(reader) = crate::fs_gate::columns::open_parquet(path).with_context(|| {
        format!(
            "read {} (delete it to rebuild this store from a full walk)",
            path.display()
        )
    })?
    else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch?;
        let worktree_id = downcast_str(&batch, "worktree_id")?;
        let rel_path = downcast_str(&batch, "rel_path")?;
        let parent_rel_path = batch
            .column_by_name("parent_rel_path")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .context("column parent_rel_path is not Utf8")?;
        let allocated_total = downcast_u64(&batch, "allocated_total")?;
        let own_allocated = downcast_u64(&batch, "own_allocated")?;
        let file_count = downcast_u32(&batch, "file_count")?;
        let entry_count = downcast_u32(&batch, "entry_count")?;
        let symlink_count = downcast_u32(&batch, "symlink_count")?;
        let mod_time_min = batch
            .column_by_name("mod_time_min")
            .and_then(|c| c.as_any().downcast_ref::<Int32Array>())
            .context("column mod_time_min is not Int32")?;
        let complete = downcast_bool(&batch, "complete")?;
        let observed_at = downcast_u64(&batch, "observed_at")?;
        for i in 0..batch.num_rows() {
            rows.push(StoredDirRow {
                worktree_id: worktree_id.value(i).to_string(),
                rel_path: rel_path.value(i).to_string(),
                parent_rel_path: if parent_rel_path.is_null(i) {
                    None
                } else {
                    Some(parent_rel_path.value(i).to_string())
                },
                allocated_total: allocated_total.value(i),
                own_allocated: own_allocated.value(i),
                file_count: file_count.value(i),
                entry_count: entry_count.value(i),
                symlink_count: symlink_count.value(i),
                mod_time_min: mod_time_min.value(i),
                complete: complete.value(i),
                observed_at: observed_at.value(i),
            });
        }
    }
    Ok(rows)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StoredFileRow {
    pub(super) worktree_id: String,
    pub(super) rel_path: String,
    pub(super) allocated: u64,
    pub(super) mod_time_min: i32,
    pub(super) observed_at: u64,
}

pub(super) fn files_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("worktree_id", DataType::Utf8, false),
        Field::new("rel_path", DataType::Utf8, false),
        Field::new("allocated", DataType::UInt64, false),
        Field::new("mod_time_min", DataType::Int32, false),
        Field::new("observed_at", DataType::UInt64, false),
    ]))
}

pub(super) fn write_file_rows(path: &Path, rows: &[StoredFileRow], zstd_level: i32) -> Result<()> {
    let schema = files_schema();
    let worktree_ids: Vec<&str> = rows.iter().map(|r| r.worktree_id.as_str()).collect();
    let rel_paths: Vec<&str> = rows.iter().map(|r| r.rel_path.as_str()).collect();
    let allocated: Vec<u64> = rows.iter().map(|r| r.allocated).collect();
    let mod_time_min: Vec<i32> = rows.iter().map(|r| r.mod_time_min).collect();
    let observed_at: Vec<u64> = rows.iter().map(|r| r.observed_at).collect();

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(worktree_ids)) as ArrayRef,
            Arc::new(StringArray::from(rel_paths)),
            Arc::new(UInt64Array::from(allocated)),
            Arc::new(Int32Array::from(mod_time_min)),
            Arc::new(UInt64Array::from(observed_at)),
        ],
    )?;
    crate::fs_gate::columns::write_parquet_atomic(
        path,
        schema,
        std::iter::once(Ok(batch)),
        zstd_level,
    )
}

pub(super) fn read_file_rows(path: &Path) -> Result<Vec<StoredFileRow>> {
    let Some(reader) = crate::fs_gate::columns::open_parquet(path).with_context(|| {
        format!(
            "read {} (delete it to rebuild this store from a full walk)",
            path.display()
        )
    })?
    else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch?;
        let worktree_id = downcast_str(&batch, "worktree_id")?;
        let rel_path = downcast_str(&batch, "rel_path")?;
        let allocated = downcast_u64(&batch, "allocated")?;
        let mod_time_min = batch
            .column_by_name("mod_time_min")
            .and_then(|c| c.as_any().downcast_ref::<Int32Array>())
            .context("column mod_time_min is not Int32")?;
        let observed_at = downcast_u64(&batch, "observed_at")?;
        for i in 0..batch.num_rows() {
            rows.push(StoredFileRow {
                worktree_id: worktree_id.value(i).to_string(),
                rel_path: rel_path.value(i).to_string(),
                allocated: allocated.value(i),
                mod_time_min: mod_time_min.value(i),
                observed_at: observed_at.value(i),
            });
        }
    }
    Ok(rows)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredExternalRow {
    detector_id: String,
    category: String,
    device: u64,
    path: String,
    bytes: u64,
    hardlinked: bool,
    present: bool,
    observed_at: u64,
    regrowth_count: u32,
}

fn external_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("detector_id", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("device", DataType::UInt64, false),
        Field::new("path", DataType::Utf8, false),
        Field::new("bytes", DataType::UInt64, false),
        Field::new("hardlinked", DataType::Boolean, false),
        Field::new("present", DataType::Boolean, false),
        Field::new("observed_at", DataType::UInt64, false),
        Field::new("regrowth_count", DataType::UInt32, false),
    ]))
}

fn write_external_rows(path: &Path, rows: &[StoredExternalRow]) -> Result<()> {
    let schema = external_schema();
    let detector_ids: Vec<&str> = rows.iter().map(|r| r.detector_id.as_str()).collect();
    let categories: Vec<&str> = rows.iter().map(|r| r.category.as_str()).collect();
    let devices: Vec<u64> = rows.iter().map(|r| r.device).collect();
    let paths: Vec<&str> = rows.iter().map(|r| r.path.as_str()).collect();
    let bytes: Vec<u64> = rows.iter().map(|r| r.bytes).collect();
    let hardlinked: Vec<bool> = rows.iter().map(|r| r.hardlinked).collect();
    let present: Vec<bool> = rows.iter().map(|r| r.present).collect();
    let observed_at: Vec<u64> = rows.iter().map(|r| r.observed_at).collect();
    let regrowth: Vec<u32> = rows.iter().map(|r| r.regrowth_count).collect();

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(detector_ids)) as ArrayRef,
            Arc::new(StringArray::from(categories)),
            Arc::new(UInt64Array::from(devices)),
            Arc::new(StringArray::from(paths)),
            Arc::new(UInt64Array::from(bytes)),
            Arc::new(BooleanArray::from(hardlinked)),
            Arc::new(BooleanArray::from(present)),
            Arc::new(UInt64Array::from(observed_at)),
            Arc::new(UInt32Array::from(regrowth)),
        ],
    )?;
    crate::fs_gate::columns::write_parquet_atomic(
        path,
        schema,
        std::iter::once(Ok(batch)),
        super::ARTIFACT_ZSTD_LEVEL,
    )
}

pub(super) fn read_external_rows(path: &Path) -> Result<Vec<StoredExternalRow>> {
    let Some(reader) = crate::fs_gate::columns::open_parquet(path).with_context(|| {
        format!(
            "read {} (delete it to rebuild this store from a full walk)",
            path.display()
        )
    })?
    else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch?;
        let detector_id = downcast_str(&batch, "detector_id")?;
        let category = downcast_str(&batch, "category")?;
        let device = downcast_u64(&batch, "device")?;
        let path_col = downcast_str(&batch, "path")?;
        let bytes = downcast_u64(&batch, "bytes")?;
        let hardlinked = downcast_bool(&batch, "hardlinked")?;
        let present = downcast_bool(&batch, "present")?;
        let observed_at = downcast_u64(&batch, "observed_at")?;
        let regrowth = downcast_u32(&batch, "regrowth_count")?;
        for i in 0..batch.num_rows() {
            rows.push(StoredExternalRow {
                detector_id: detector_id.value(i).to_string(),
                category: category.value(i).to_string(),
                device: device.value(i),
                path: path_col.value(i).to_string(),
                bytes: bytes.value(i),
                hardlinked: hardlinked.value(i),
                present: present.value(i),
                observed_at: observed_at.value(i),
                regrowth_count: regrowth.value(i),
            });
        }
    }
    Ok(rows)
}

/// One row per directory a unit's folded measurement listed, plus one
/// root row (`rel_dir == ""`) carrying the measurement itself.
///
/// This is a *measurement cache*, not history: it never feeds growth,
/// tombstones or regrowth, and deleting it only costs one full
/// re-measurement. It is per **directory**, never per file -- the
/// handoff forbids a per-file persistent inventory, and a directory's
/// own `mtime`/`ctime` already move when an entry inside it is created,
/// removed or renamed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldedRow {
    pub unit_path: String,
    pub rel_dir: String,
    pub mtime_ns: i64,
    pub ctime_ns: i64,
    /// Root row only: the folded byte total, whether any member was
    /// hardlinked, the newest member mtime, when it was measured, and a
    /// digest of the exclusion list it was measured under.
    pub bytes: u64,
    pub hardlinked: bool,
    pub mtime_max: u64,
    pub observed_at: u64,
    pub exclusions: String,
}

fn folded_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("unit_path", DataType::Utf8, false),
        Field::new("rel_dir", DataType::Utf8, false),
        Field::new("mtime_ns", DataType::Int64, false),
        Field::new("ctime_ns", DataType::Int64, false),
        Field::new("bytes", DataType::UInt64, false),
        Field::new("hardlinked", DataType::Boolean, false),
        Field::new("mtime_max", DataType::UInt64, false),
        Field::new("observed_at", DataType::UInt64, false),
        Field::new("exclusions", DataType::Utf8, false),
    ]))
}

pub(super) fn write_folded_rows(path: &Path, rows: &[FoldedRow]) -> Result<()> {
    let schema = folded_schema();
    let unit_path: Vec<&str> = rows.iter().map(|r| r.unit_path.as_str()).collect();
    let rel_dir: Vec<&str> = rows.iter().map(|r| r.rel_dir.as_str()).collect();
    let mtime_ns: Vec<i64> = rows.iter().map(|r| r.mtime_ns).collect();
    let ctime_ns: Vec<i64> = rows.iter().map(|r| r.ctime_ns).collect();
    let bytes: Vec<u64> = rows.iter().map(|r| r.bytes).collect();
    let hardlinked: Vec<bool> = rows.iter().map(|r| r.hardlinked).collect();
    let mtime_max: Vec<u64> = rows.iter().map(|r| r.mtime_max).collect();
    let observed_at: Vec<u64> = rows.iter().map(|r| r.observed_at).collect();
    let exclusions: Vec<&str> = rows.iter().map(|r| r.exclusions.as_str()).collect();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(unit_path)) as ArrayRef,
            Arc::new(StringArray::from(rel_dir)),
            Arc::new(Int64Array::from(mtime_ns)),
            Arc::new(Int64Array::from(ctime_ns)),
            Arc::new(UInt64Array::from(bytes)),
            Arc::new(BooleanArray::from(hardlinked)),
            Arc::new(UInt64Array::from(mtime_max)),
            Arc::new(UInt64Array::from(observed_at)),
            Arc::new(StringArray::from(exclusions)),
        ],
    )?;
    crate::fs_gate::columns::write_parquet_atomic(
        path,
        schema,
        std::iter::once(Ok(batch)),
        super::ARTIFACT_ZSTD_LEVEL,
    )
}

pub(super) fn read_folded_rows(path: &Path) -> Result<Vec<FoldedRow>> {
    let Some(reader) = crate::fs_gate::columns::open_parquet(path)? else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch?;
        let unit_path = downcast_str(&batch, "unit_path")?;
        let rel_dir = downcast_str(&batch, "rel_dir")?;
        let mtime_ns = downcast_i64(&batch, "mtime_ns")?;
        let ctime_ns = downcast_i64(&batch, "ctime_ns")?;
        let bytes = downcast_u64(&batch, "bytes")?;
        let hardlinked = downcast_bool(&batch, "hardlinked")?;
        let mtime_max = downcast_u64(&batch, "mtime_max")?;
        let observed_at = downcast_u64(&batch, "observed_at")?;
        let exclusions = downcast_str(&batch, "exclusions")?;
        for i in 0..batch.num_rows() {
            rows.push(FoldedRow {
                unit_path: unit_path.value(i).to_string(),
                rel_dir: rel_dir.value(i).to_string(),
                mtime_ns: mtime_ns.value(i),
                ctime_ns: ctime_ns.value(i),
                bytes: bytes.value(i),
                hardlinked: hardlinked.value(i),
                mtime_max: mtime_max.value(i),
                observed_at: observed_at.value(i),
                exclusions: exclusions.value(i).to_string(),
            });
        }
    }
    Ok(rows)
}

// ---------------------------------------------------------------------
// The typed history transitions
// ---------------------------------------------------------------------

impl StoredRow {
    pub(super) fn project_id(&self) -> &str {
        &self.project_id
    }
    pub(super) fn worktree_id(&self) -> &str {
        &self.worktree_id
    }
    pub(super) fn kind(&self) -> &str {
        &self.kind
    }
    pub(super) fn rel_path(&self) -> &str {
        &self.rel_path
    }
    pub(super) fn bytes(&self) -> u64 {
        self.bytes
    }
    pub(super) fn dedup_stale(&self) -> bool {
        self.dedup_stale
    }
    pub(super) fn local_bytes(&self) -> u64 {
        self.local_bytes
    }
    pub(super) fn mtime_max(&self) -> u64 {
        self.mtime_max
    }
    pub(super) fn hardlinked(&self) -> bool {
        self.hardlinked
    }
    pub(super) fn present(&self) -> bool {
        self.present
    }
    pub(super) fn observed_at(&self) -> u64 {
        self.observed_at
    }
    pub(super) fn regrowth_count(&self) -> u32 {
        self.regrowth_count
    }
    pub(super) fn key(&self) -> String {
        super::row_key(
            &self.project_id,
            &self.worktree_id,
            &self.kind,
            &self.rel_path,
        )
    }

    /// A row as stored, for this module's own tests and the compaction
    /// tests in `growth`.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn for_test(
        project_id: &str,
        worktree_id: &str,
        kind: &str,
        rel_path: &str,
        bytes: u64,
        present: bool,
        observed_at: u64,
        regrowth_count: u32,
    ) -> StoredRow {
        StoredRow {
            project_id: project_id.into(),
            worktree_id: worktree_id.into(),
            kind: kind.into(),
            rel_path: rel_path.into(),
            bytes,
            local_bytes: bytes,
            mtime_max: bytes,
            hardlinked: false,
            dedup_stale: false,
            present,
            observed_at,
            regrowth_count,
        }
    }

    /// Writes rows as a delta file, for the compaction tests.
    #[cfg(test)]
    pub(super) fn write_for_test(path: &Path, rows: &[StoredRow]) -> Result<()> {
        write_rows(path, rows)
    }
}

impl StoredExternalRow {
    pub(crate) fn detector_id(&self) -> &str {
        &self.detector_id
    }
    pub(crate) fn category(&self) -> &str {
        &self.category
    }
    pub(crate) fn device(&self) -> u64 {
        self.device
    }
    pub(crate) fn path(&self) -> &str {
        &self.path
    }
    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }
    pub(crate) fn observed_at(&self) -> u64 {
        self.observed_at
    }
    pub(crate) fn regrowth_count(&self) -> u32 {
        self.regrowth_count
    }
    pub(crate) fn key(&self) -> String {
        super::external_row_key(&self.detector_id, &self.category, self.device, &self.path)
    }
}

/// A claim that this observation owns one stored row: the only thing
/// [`ArtifactHistory::tombstone`] and [`ExternalHistory::tombstone`]
/// accept. Constructible only here, by an ownership's `claim`.
#[must_use = "an ownership claim is only meaningful when it is spent on the tombstone it licenses"]
#[derive(Debug)]
pub(super) struct Owned<'a> {
    key: &'a str,
}

impl super::ObservationOwnership {
    /// `Some` exactly when this observation owns the stored row `key`
    /// (its key family, inside a region this pass covered and observed):
    /// the one way to obtain the [`Owned`] a tombstone needs.
    pub(super) fn claim<'a>(&self, key: &'a str) -> Option<Owned<'a>> {
        self.owns(key).then_some(Owned { key })
    }
}

/// Which artifact rows a walk may tombstone: every row of the walked
/// store except those whose worktree could not be confirmed this pass
/// (#42 -- absence reflects lost access, not deletion).
pub(super) struct ArtifactOwnership<'u> {
    pub(super) unconfirmed_worktrees: &'u HashSet<String>,
}

impl ArtifactOwnership<'_> {
    pub(super) fn claim<'a>(&self, history: &ArtifactHistory, key: &'a str) -> Option<Owned<'a>> {
        let row = history.current.get(key)?;
        (!self.unconfirmed_worktrees.contains(&row.worktree_id)).then_some(Owned { key })
    }
}

/// `current.parquet` of the artifact store, loaded, plus the reverse
/// delta this observation is accumulating. The only way to change a
/// stored artifact row.
pub(super) struct ArtifactHistory {
    dir: PathBuf,
    current: HashMap<String, StoredRow>,
    deltas: Vec<StoredRow>,
    changed: bool,
}

impl ArtifactHistory {
    pub(super) fn load(dir: &Path) -> Result<ArtifactHistory> {
        let current = read_rows(&super::current_path(dir))?
            .into_iter()
            .map(|r| (r.key(), r))
            .collect();
        Ok(ArtifactHistory {
            dir: dir.to_path_buf(),
            current,
            deltas: Vec::new(),
            changed: false,
        })
    }

    pub(super) fn row(&self, key: &str) -> Option<&StoredRow> {
        self.current.get(key)
    }

    /// Keys of rows currently present that this pass did not see: the
    /// candidates a caller may *try* to claim.
    pub(super) fn unseen_present(&self, seen: &HashSet<String>) -> Vec<String> {
        let mut out: Vec<String> = self
            .current
            .iter()
            .filter(|(k, r)| r.present && !seen.contains(*k))
            .map(|(k, _)| k.clone())
            .collect();
        out.sort();
        out
    }

    /// Records one observed row. A change to a known row (bytes,
    /// presence, dedup state) pushes its previous value onto the delta;
    /// a tombstoned row seen again counts one regrowth. A newly
    /// discovered row is simply the first known value: no synthetic
    /// "previously absent" delta, which would plant a fabricated
    /// (bytes=0, present=false) history point that can tie with (or beat)
    /// a real historical value in `growth_since`.
    pub(super) fn observe(&mut self, obs: &super::Observed, observed_at: u64) {
        match self.current.get_mut(&obs.key) {
            Some(prev) => {
                // Whether the unit holds hardlinked files is a property
                // of the walk, not of its byte total: refresh it even
                // when the bytes did not move, or a row first recorded
                // under the conservative default would keep that default
                // forever and never regain the fast path.
                if prev.hardlinked != obs.hardlinked
                    || prev.dedup_stale != obs.dedup_stale
                    || prev.mtime_max != obs.mtime_max
                    || prev.local_bytes != obs.local_bytes
                {
                    self.changed = true;
                }
                let changed =
                    prev.bytes != obs.bytes || !prev.present || prev.dedup_stale != obs.dedup_stale;
                if changed {
                    self.changed = true;
                    let regrowth_count = if !prev.present {
                        prev.regrowth_count + 1
                    } else {
                        prev.regrowth_count
                    };
                    // The delta's timestamp is when this *old* value was
                    // itself last confirmed (`prev.observed_at`), not this
                    // observation's: tagging it with the current one
                    // collides with the new `current` row's timestamp and
                    // lets an arbitrary value win `growth_since`'s
                    // nearest-timestamp lookup.
                    self.deltas.push(prev.clone());
                    prev.bytes = obs.bytes;
                    prev.local_bytes = obs.local_bytes;
                    prev.present = true;
                    prev.observed_at = observed_at;
                    prev.regrowth_count = regrowth_count;
                }
                prev.dedup_stale = obs.dedup_stale;
                prev.hardlinked = obs.hardlinked;
                prev.mtime_max = obs.mtime_max;
                prev.local_bytes = obs.local_bytes;
            }
            None => {
                self.changed = true;
                self.current.insert(
                    obs.key.clone(),
                    StoredRow {
                        project_id: obs.project_id.clone(),
                        worktree_id: obs.worktree_id.clone(),
                        kind: obs.kind.clone(),
                        rel_path: obs.rel_path.clone(),
                        bytes: obs.bytes,
                        local_bytes: obs.local_bytes,
                        mtime_max: obs.mtime_max,
                        hardlinked: obs.hardlinked,
                        dedup_stale: obs.dedup_stale,
                        present: true,
                        observed_at,
                        regrowth_count: 0,
                    },
                );
            }
        }
    }

    /// Marks an owned, present row absent (kept in the store, so a later
    /// reappearance counts as regrowth), pushing its previous value onto
    /// the delta.
    pub(super) fn tombstone(&mut self, owned: Owned<'_>, observed_at: u64) {
        let Some(row) = self.current.get_mut(owned.key) else {
            return;
        };
        if !row.present {
            return;
        }
        self.deltas.push(row.clone());
        row.present = false;
        row.bytes = 0;
        row.observed_at = observed_at;
        self.changed = true;
    }

    /// Writes this observation: the reverse delta first (if anything
    /// changed a known row), then `current.parquet` (if anything changed
    /// at all). Never the current table without its delta.
    pub(super) fn commit(self) -> Result<()> {
        if !self.deltas.is_empty() {
            write_rows(&super::next_delta_path(&self.dir), &self.deltas)?;
        }
        if self.changed {
            let mut rows: Vec<StoredRow> = self.current.into_values().collect();
            rows.sort_by(|a, b| {
                (&a.project_id, &a.worktree_id, &a.kind, &a.rel_path).cmp(&(
                    &b.project_id,
                    &b.worktree_id,
                    &b.kind,
                    &b.rel_path,
                ))
            });
            write_rows(&super::current_path(&self.dir), &rows)?;
        }
        Ok(())
    }
}

/// Merges `files` (delta files) into one, dropping rows older than
/// `horizon`, and removes the sources once the replacement is published.
pub(super) fn compact_artifact_deltas(dir: &Path, files: &[PathBuf], horizon: u64) -> Result<()> {
    let mut merged: Vec<StoredRow> = Vec::new();
    for path in files {
        for row in read_rows(path)? {
            if row.observed_at >= horizon {
                merged.push(row);
            }
        }
    }
    if !merged.is_empty() {
        merged.sort_by(|a, b| {
            (
                &a.project_id,
                &a.worktree_id,
                &a.kind,
                &a.rel_path,
                a.observed_at,
            )
                .cmp(&(
                    &b.project_id,
                    &b.worktree_id,
                    &b.kind,
                    &b.rel_path,
                    b.observed_at,
                ))
        });
        write_rows(&super::next_delta_path(dir), &merged)?;
    }
    // Publish the completed replacement before retiring any source file.
    for path in files {
        crate::fs_gate::columns::retire(path)?;
    }
    Ok(())
}

/// The external-unit twin of [`ArtifactHistory`].
pub(super) struct ExternalHistory {
    dir: PathBuf,
    current: HashMap<String, StoredExternalRow>,
    deltas: Vec<StoredExternalRow>,
    changed: bool,
}

impl ExternalHistory {
    pub(super) fn load(dir: &Path) -> Result<ExternalHistory> {
        let current = read_external_rows(&super::external_current_path(dir))?
            .into_iter()
            .map(|r| (r.key(), r))
            .collect();
        Ok(ExternalHistory {
            dir: dir.to_path_buf(),
            current,
            deltas: Vec::new(),
            changed: false,
        })
    }

    pub(super) fn row(&self, key: &str) -> Option<&StoredExternalRow> {
        self.current.get(key)
    }

    pub(super) fn unseen_present(&self, seen: &HashSet<String>) -> Vec<String> {
        let mut out: Vec<String> = self
            .current
            .iter()
            .filter(|(k, r)| r.present && !seen.contains(*k))
            .map(|(k, _)| k.clone())
            .collect();
        out.sort();
        out
    }

    pub(super) fn observe(&mut self, obs: &super::ObservedExternal, observed_at: u64) {
        match self.current.get_mut(&obs.key) {
            Some(prev) => {
                let changed = prev.bytes != obs.bytes || !prev.present;
                if changed {
                    self.changed = true;
                    let regrowth_count = if !prev.present {
                        prev.regrowth_count + 1
                    } else {
                        prev.regrowth_count
                    };
                    self.deltas.push(prev.clone());
                    prev.bytes = obs.bytes;
                    prev.present = true;
                    prev.observed_at = observed_at;
                    prev.regrowth_count = regrowth_count;
                }
                if prev.hardlinked != obs.hardlinked {
                    self.changed = true;
                }
                prev.hardlinked = obs.hardlinked;
            }
            None => {
                self.changed = true;
                self.current.insert(
                    obs.key.clone(),
                    StoredExternalRow {
                        detector_id: obs.detector_id.clone(),
                        category: obs.category.clone(),
                        device: obs.device,
                        path: obs.path.clone(),
                        bytes: obs.bytes,
                        hardlinked: obs.hardlinked,
                        present: true,
                        observed_at,
                        regrowth_count: 0,
                    },
                );
            }
        }
    }

    pub(super) fn tombstone(&mut self, owned: Owned<'_>, observed_at: u64) {
        let Some(row) = self.current.get_mut(owned.key) else {
            return;
        };
        if !row.present {
            return;
        }
        self.deltas.push(row.clone());
        row.present = false;
        row.bytes = 0;
        row.observed_at = observed_at;
        self.changed = true;
    }

    pub(super) fn commit(self) -> Result<()> {
        if !self.deltas.is_empty() {
            let seq_path = super::next_seq_path(&super::external_deltas_dir(&self.dir), "delta-");
            write_external_rows(&seq_path, &self.deltas)?;
        }
        if self.changed {
            let mut rows: Vec<StoredExternalRow> = self.current.into_values().collect();
            rows.sort_by(|a, b| {
                (&a.detector_id, &a.category, a.device, &a.path).cmp(&(
                    &b.detector_id,
                    &b.category,
                    b.device,
                    &b.path,
                ))
            });
            write_external_rows(&super::external_current_path(&self.dir), &rows)?;
        }
        Ok(())
    }
}

/// [`compact_artifact_deltas`] for the external store.
pub(super) fn compact_external_deltas(dir: &Path, files: &[PathBuf], horizon: u64) -> Result<()> {
    let mut merged: Vec<StoredExternalRow> = Vec::new();
    for path in files {
        for row in read_external_rows(path)? {
            if row.observed_at >= horizon {
                merged.push(row);
            }
        }
    }
    if !merged.is_empty() {
        let seq_path = super::next_seq_path(&super::external_deltas_dir(dir), "delta-");
        write_external_rows(&seq_path, &merged)?;
    }
    for path in files {
        crate::fs_gate::columns::retire(path)?;
    }
    Ok(())
}
