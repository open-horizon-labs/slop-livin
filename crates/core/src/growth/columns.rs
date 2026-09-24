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
    Array, ArrayRef, BooleanArray, Float64Array, Int32Array, Int64Array, RecordBatch, StringArray,
    UInt32Array, UInt64Array,
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
    /// Ecosystem tag that generates this artifact (`rs`, `js`, ...);
    /// `None` for a row with no ecosystem (`Source`/`Git`/etc.) and for
    /// stores written before this column existed (R15 item 3: render's
    /// `ArtifactRow::ecosystem` was the one artifact-render field the
    /// existing current-artifact table did not already carry).
    ecosystem: Option<String>,
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
        Field::new("ecosystem", DataType::Utf8, true),
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
    let ecosystem: Vec<Option<&str>> = rows.iter().map(|r| r.ecosystem.as_deref()).collect();

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
            Arc::new(StringArray::from(ecosystem)),
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
        // Stores written before R15 item 3 have no ecosystem column.
        let ecosystem = batch
            .column_by_name("ecosystem")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>());
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
                ecosystem: ecosystem.and_then(|c| c.is_valid(i).then(|| c.value(i).to_string())),
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
// unowned/current.parquet -- the unowned/remainder rows of one volume
// (#R10 item 1). Folded directory rows, never per file (`walk.rs`
// folds every direct unowned file into one row per containing
// directory before this table ever sees them). Like `FoldedRow`, this
// is a measurement cache, not history: unowned rows carry no growth or
// regrowth semantics and are replaced wholesale by a full walk, so the
// whole file is rewritten each observation rather than reverse-delta
// compacted. Complex per-row fields (`containers`, `shared_with`,
// `evidence`) are JSON-encoded *into a Parquet Utf8 cell*, not a JSON
// file on disk -- the store-is-Parquet rule is about the file format
// under `SWAMP_DIR`, not about every cell's encoding.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct StoredUnownedRow {
    pub path_or_object: String,
    pub bytes: u64,
    pub reason: String,
    pub shared_bytes: Option<u64>,
    pub note: Option<String>,
    pub docker_kind: Option<String>,
    pub created_at: Option<String>,
    /// JSON-encoded `Vec<String>`.
    pub containers_json: String,
    /// JSON-encoded `Vec<String>`.
    pub shared_with_json: String,
    pub dangling: bool,
    /// JSON-encoded `Vec<crate::evidence::Evidence>`.
    pub evidence_json: String,
}

fn unowned_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("path_or_object", DataType::Utf8, false),
        Field::new("bytes", DataType::UInt64, false),
        Field::new("reason", DataType::Utf8, false),
        Field::new("shared_bytes", DataType::UInt64, true),
        Field::new("note", DataType::Utf8, true),
        Field::new("docker_kind", DataType::Utf8, true),
        Field::new("created_at", DataType::Utf8, true),
        Field::new("containers_json", DataType::Utf8, false),
        Field::new("shared_with_json", DataType::Utf8, false),
        Field::new("dangling", DataType::Boolean, false),
        Field::new("evidence_json", DataType::Utf8, false),
    ]))
}

pub(super) fn write_unowned_rows(path: &Path, rows: &[StoredUnownedRow]) -> Result<()> {
    let schema = unowned_schema();
    let path_or_object: Vec<&str> = rows.iter().map(|r| r.path_or_object.as_str()).collect();
    let bytes: Vec<u64> = rows.iter().map(|r| r.bytes).collect();
    let reason: Vec<&str> = rows.iter().map(|r| r.reason.as_str()).collect();
    let shared_bytes: Vec<Option<u64>> = rows.iter().map(|r| r.shared_bytes).collect();
    let note: Vec<Option<&str>> = rows.iter().map(|r| r.note.as_deref()).collect();
    let docker_kind: Vec<Option<&str>> = rows.iter().map(|r| r.docker_kind.as_deref()).collect();
    let created_at: Vec<Option<&str>> = rows.iter().map(|r| r.created_at.as_deref()).collect();
    let containers_json: Vec<&str> = rows.iter().map(|r| r.containers_json.as_str()).collect();
    let shared_with_json: Vec<&str> = rows.iter().map(|r| r.shared_with_json.as_str()).collect();
    let dangling: Vec<bool> = rows.iter().map(|r| r.dangling).collect();
    let evidence_json: Vec<&str> = rows.iter().map(|r| r.evidence_json.as_str()).collect();

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(path_or_object)) as ArrayRef,
            Arc::new(UInt64Array::from(bytes)),
            Arc::new(StringArray::from(reason)),
            Arc::new(UInt64Array::from(shared_bytes)),
            Arc::new(StringArray::from(note)),
            Arc::new(StringArray::from(docker_kind)),
            Arc::new(StringArray::from(created_at)),
            Arc::new(StringArray::from(containers_json)),
            Arc::new(StringArray::from(shared_with_json)),
            Arc::new(BooleanArray::from(dangling)),
            Arc::new(StringArray::from(evidence_json)),
        ],
    )?;
    crate::fs_gate::columns::write_parquet_atomic(
        path,
        schema,
        std::iter::once(Ok(batch)),
        super::ARTIFACT_ZSTD_LEVEL,
    )
}

pub(super) fn read_unowned_rows(path: &Path) -> Result<Vec<StoredUnownedRow>> {
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
        let path_or_object = downcast_str(&batch, "path_or_object")?;
        let bytes = downcast_u64(&batch, "bytes")?;
        let reason = downcast_str(&batch, "reason")?;
        let shared_bytes = batch
            .column_by_name("shared_bytes")
            .and_then(|c| c.as_any().downcast_ref::<UInt64Array>())
            .context("column shared_bytes is not UInt64")?;
        let note = batch
            .column_by_name("note")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .context("column note is not Utf8")?;
        let docker_kind = batch
            .column_by_name("docker_kind")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .context("column docker_kind is not Utf8")?;
        let created_at = batch
            .column_by_name("created_at")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .context("column created_at is not Utf8")?;
        let containers_json = downcast_str(&batch, "containers_json")?;
        let shared_with_json = downcast_str(&batch, "shared_with_json")?;
        let dangling = downcast_bool(&batch, "dangling")?;
        let evidence_json = downcast_str(&batch, "evidence_json")?;
        for i in 0..batch.num_rows() {
            rows.push(StoredUnownedRow {
                path_or_object: path_or_object.value(i).to_string(),
                bytes: bytes.value(i),
                reason: reason.value(i).to_string(),
                shared_bytes: shared_bytes.is_valid(i).then(|| shared_bytes.value(i)),
                note: note.is_valid(i).then(|| note.value(i).to_string()),
                docker_kind: docker_kind
                    .is_valid(i)
                    .then(|| docker_kind.value(i).to_string()),
                created_at: created_at
                    .is_valid(i)
                    .then(|| created_at.value(i).to_string()),
                containers_json: containers_json.value(i).to_string(),
                shared_with_json: shared_with_json.value(i).to_string(),
                dangling: dangling.value(i),
                evidence_json: evidence_json.value(i).to_string(),
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
    pub(super) fn ecosystem(&self) -> Option<&str> {
        self.ecosystem.as_deref()
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
            ecosystem: None,
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
                    || prev.ecosystem != obs.ecosystem
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
                prev.ecosystem = obs.ecosystem.clone();
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
                        ecosystem: obs.ecosystem.clone(),
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

// ---------------------------------------------------------------------
// report_rows.parquet -- the rendered-row data `swamp report` and the
// TUI need to render without walking anything (R12: `swamp report` is a
// pure read; `swamp observe` is the only scanner). One row per scope
// key, replaced wholesale by every `observe` that covers that scope --
// like `unowned.parquet`/`folded.parquet`, this is a measurement cache,
// not reverse-delta history. The rendered `Report` (evidence, tracking
// and Docker joins already attached by the pipeline that produced it),
// per-root coverage, external units, agent units and store interiors
// are each JSON-encoded into their own Parquet `Utf8` cell -- a cell
// encoding, not a JSON file on disk, exactly like `unowned.parquet`'s
// `evidence_json`/`containers_json` columns.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct StoredReportSnapshotRow {
    pub scope_key: String,
    pub observed_at: u64,
    /// JSON-encoded `crate::report::Report`.
    pub report_json: String,
    /// JSON-encoded `Vec<crate::coverage::RootCoverage>`.
    pub coverage_json: String,
    /// JSON-encoded `Vec<crate::external::ExternalUnit>`.
    pub external_units_json: String,
    /// JSON-encoded `Vec<crate::agents::AgentUnit>`.
    pub agent_units_json: String,
    /// JSON-encoded `Vec<crate::artifact::NestedArtifact>`.
    pub store_interiors_json: String,
}

fn report_snapshot_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("scope_key", DataType::Utf8, false),
        Field::new("observed_at", DataType::UInt64, false),
        Field::new("report_json", DataType::Utf8, false),
        Field::new("coverage_json", DataType::Utf8, false),
        Field::new("external_units_json", DataType::Utf8, false),
        Field::new("agent_units_json", DataType::Utf8, false),
        Field::new("store_interiors_json", DataType::Utf8, false),
    ]))
}

pub(super) fn write_report_snapshot_rows(
    path: &Path,
    rows: &[StoredReportSnapshotRow],
) -> Result<()> {
    let schema = report_snapshot_schema();
    let scope_key: Vec<&str> = rows.iter().map(|r| r.scope_key.as_str()).collect();
    let observed_at: Vec<u64> = rows.iter().map(|r| r.observed_at).collect();
    let report_json: Vec<&str> = rows.iter().map(|r| r.report_json.as_str()).collect();
    let coverage_json: Vec<&str> = rows.iter().map(|r| r.coverage_json.as_str()).collect();
    let external_units_json: Vec<&str> = rows
        .iter()
        .map(|r| r.external_units_json.as_str())
        .collect();
    let agent_units_json: Vec<&str> = rows.iter().map(|r| r.agent_units_json.as_str()).collect();
    let store_interiors_json: Vec<&str> = rows
        .iter()
        .map(|r| r.store_interiors_json.as_str())
        .collect();

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(scope_key)) as ArrayRef,
            Arc::new(UInt64Array::from(observed_at)),
            Arc::new(StringArray::from(report_json)),
            Arc::new(StringArray::from(coverage_json)),
            Arc::new(StringArray::from(external_units_json)),
            Arc::new(StringArray::from(agent_units_json)),
            Arc::new(StringArray::from(store_interiors_json)),
        ],
    )?;
    crate::fs_gate::columns::write_parquet_atomic(
        path,
        schema,
        std::iter::once(Ok(batch)),
        super::ARTIFACT_ZSTD_LEVEL,
    )
}

pub(super) fn read_report_snapshot_rows(path: &Path) -> Result<Vec<StoredReportSnapshotRow>> {
    let Some(reader) = crate::fs_gate::columns::open_parquet(path).with_context(|| {
        format!(
            "read {} (delete it; the next `swamp observe` rebuilds it)",
            path.display()
        )
    })?
    else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch?;
        let scope_key = downcast_str(&batch, "scope_key")?;
        let observed_at = downcast_u64(&batch, "observed_at")?;
        let report_json = downcast_str(&batch, "report_json")?;
        let coverage_json = downcast_str(&batch, "coverage_json")?;
        let external_units_json = downcast_str(&batch, "external_units_json")?;
        let agent_units_json = downcast_str(&batch, "agent_units_json")?;
        let store_interiors_json = downcast_str(&batch, "store_interiors_json")?;
        for i in 0..batch.num_rows() {
            rows.push(StoredReportSnapshotRow {
                scope_key: scope_key.value(i).to_string(),
                observed_at: observed_at.value(i),
                report_json: report_json.value(i).to_string(),
                coverage_json: coverage_json.value(i).to_string(),
                external_units_json: external_units_json.value(i).to_string(),
                agent_units_json: agent_units_json.value(i).to_string(),
                store_interiors_json: store_interiors_json.value(i).to_string(),
            });
        }
    }
    Ok(rows)
}

// ---------------------------------------------------------------------
// protect.parquet -- the human keep list (`crate::protection`; R14 item
// A). Two typed columns, no JSON cell: the whole point of moving it off
// `agent_protect.json`. `crate::protection` owns the semantics (the one
// `conflict` predicate, absolute-path validation, idempotent add); this
// module only owns the Arrow schema, like every other table here.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StoredProtectRow {
    pub(crate) path: String,
    pub(crate) added_at: u64,
}

fn protect_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("path", DataType::Utf8, false),
        Field::new("added_at", DataType::UInt64, false),
    ]))
}

pub(crate) fn write_protect_rows(path: &Path, rows: &[StoredProtectRow]) -> Result<()> {
    let schema = protect_schema();
    let paths: Vec<&str> = rows.iter().map(|r| r.path.as_str()).collect();
    let added_at: Vec<u64> = rows.iter().map(|r| r.added_at).collect();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(paths)) as ArrayRef,
            Arc::new(UInt64Array::from(added_at)),
        ],
    )?;
    crate::fs_gate::columns::write_parquet_atomic(
        path,
        schema,
        std::iter::once(Ok(batch)),
        crate::fs_gate::columns::DEFAULT_ZSTD_LEVEL,
    )
}

pub(crate) fn read_protect_rows(path: &Path) -> Result<Vec<StoredProtectRow>> {
    let Some(reader) = crate::fs_gate::columns::open_parquet(path).with_context(|| {
        format!(
            "read {} (delete it to reset the human keep list)",
            path.display()
        )
    })?
    else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch?;
        let path_col = downcast_str(&batch, "path")?;
        let added_at_col = downcast_u64(&batch, "added_at")?;
        for i in 0..batch.num_rows() {
            rows.push(StoredProtectRow {
                path: path_col.value(i).to_string(),
                added_at: added_at_col.value(i),
            });
        }
    }
    Ok(rows)
}

// ---------------------------------------------------------------------
// projects.parquet / worktrees.parquet / worktree_facts.parquet -- R15
// item 2/~10 of the JSON-in-the-store decomposition begun by R14 item A
// (`protect.parquet`). One row per `ProjectRow`/`WorktreeRow`, scope-wide
// (top-level `swamp_dir`, keyed by `scope_key` exactly like
// `report_rows.parquet`, since one scope can span several volumes) and
// rewritten wholesale for that key by every `observe` that covers it --
// a measurement cache, not reverse-delta history, like
// `report_rows.parquet` itself. `worktree_facts.parquet` is the child
// table for the two list-valued facts a `WorktreeRow` carries
// (`signals`, `merge_complete.terms`): one row per list entry, ordered
// by `seq` so the original `Vec` order is recoverable exactly.
//
// What is deliberately NOT here: a `WorktreeRow`'s `artifacts` (their
// own table, extended in this same slice -- see the `ecosystem` column
// added to `StoredRow` above) and everything CHUNK_R15 named as later
// slices (nested artifacts, evidence, external/agent units, coverage,
// series, summary) -- those stay in `report_rows.parquet`'s JSON cell
// for now; `report::report_scope_from_store` reads this table for a
// `WorktreeRow`'s own scalars and overlays the remaining fields from
// that snapshot by key.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StoredProjectRow {
    pub(crate) scope_key: String,
    pub(crate) project_id: String,
    pub(crate) name: String,
    /// `|`-joined ecosystem tags, in their original order (`|` never
    /// appears in a tag).
    pub(crate) ecosystems: String,
    pub(crate) remote: Option<String>,
    pub(crate) bytes: u64,
    pub(crate) local_bytes: u64,
    pub(crate) allocated_bytes: u64,
    pub(crate) growth_bytes: Option<i64>,
    pub(crate) regrowth_count: u32,
    pub(crate) worktree_count: u32,
    pub(crate) observed_at: u64,
}

fn projects_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("scope_key", DataType::Utf8, false),
        Field::new("project_id", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("ecosystems", DataType::Utf8, false),
        Field::new("remote", DataType::Utf8, true),
        Field::new("bytes", DataType::UInt64, false),
        Field::new("local_bytes", DataType::UInt64, false),
        Field::new("allocated_bytes", DataType::UInt64, false),
        Field::new("growth_bytes", DataType::Int64, true),
        Field::new("regrowth_count", DataType::UInt32, false),
        Field::new("worktree_count", DataType::UInt32, false),
        Field::new("observed_at", DataType::UInt64, false),
    ]))
}

pub(crate) fn write_project_rows(path: &Path, rows: &[StoredProjectRow]) -> Result<()> {
    let schema = projects_schema();
    let scope_key: Vec<&str> = rows.iter().map(|r| r.scope_key.as_str()).collect();
    let project_id: Vec<&str> = rows.iter().map(|r| r.project_id.as_str()).collect();
    let name: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    let ecosystems: Vec<&str> = rows.iter().map(|r| r.ecosystems.as_str()).collect();
    let remote: Vec<Option<&str>> = rows.iter().map(|r| r.remote.as_deref()).collect();
    let bytes: Vec<u64> = rows.iter().map(|r| r.bytes).collect();
    let local_bytes: Vec<u64> = rows.iter().map(|r| r.local_bytes).collect();
    let allocated_bytes: Vec<u64> = rows.iter().map(|r| r.allocated_bytes).collect();
    let growth_bytes: Vec<Option<i64>> = rows.iter().map(|r| r.growth_bytes).collect();
    let regrowth_count: Vec<u32> = rows.iter().map(|r| r.regrowth_count).collect();
    let worktree_count: Vec<u32> = rows.iter().map(|r| r.worktree_count).collect();
    let observed_at: Vec<u64> = rows.iter().map(|r| r.observed_at).collect();

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(scope_key)) as ArrayRef,
            Arc::new(StringArray::from(project_id)),
            Arc::new(StringArray::from(name)),
            Arc::new(StringArray::from(ecosystems)),
            Arc::new(StringArray::from(remote)),
            Arc::new(UInt64Array::from(bytes)),
            Arc::new(UInt64Array::from(local_bytes)),
            Arc::new(UInt64Array::from(allocated_bytes)),
            Arc::new(Int64Array::from(growth_bytes)),
            Arc::new(UInt32Array::from(regrowth_count)),
            Arc::new(UInt32Array::from(worktree_count)),
            Arc::new(UInt64Array::from(observed_at)),
        ],
    )?;
    crate::fs_gate::columns::write_parquet_atomic(
        path,
        schema,
        std::iter::once(Ok(batch)),
        super::ARTIFACT_ZSTD_LEVEL,
    )
}

pub(crate) fn read_project_rows(path: &Path) -> Result<Vec<StoredProjectRow>> {
    let Some(reader) = crate::fs_gate::columns::open_parquet(path).with_context(|| {
        format!(
            "read {} (delete it; the next `swamp observe` rebuilds it)",
            path.display()
        )
    })?
    else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch?;
        let scope_key = downcast_str(&batch, "scope_key")?;
        let project_id = downcast_str(&batch, "project_id")?;
        let name = downcast_str(&batch, "name")?;
        let ecosystems = downcast_str(&batch, "ecosystems")?;
        let remote = batch
            .column_by_name("remote")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .context("column remote is not Utf8")?;
        let bytes = downcast_u64(&batch, "bytes")?;
        let local_bytes = downcast_u64(&batch, "local_bytes")?;
        let allocated_bytes = downcast_u64(&batch, "allocated_bytes")?;
        let growth_bytes = batch
            .column_by_name("growth_bytes")
            .and_then(|c| c.as_any().downcast_ref::<Int64Array>())
            .context("column growth_bytes is not Int64")?;
        let regrowth_count = downcast_u32(&batch, "regrowth_count")?;
        let worktree_count = downcast_u32(&batch, "worktree_count")?;
        let observed_at = downcast_u64(&batch, "observed_at")?;
        for i in 0..batch.num_rows() {
            rows.push(StoredProjectRow {
                scope_key: scope_key.value(i).to_string(),
                project_id: project_id.value(i).to_string(),
                name: name.value(i).to_string(),
                ecosystems: ecosystems.value(i).to_string(),
                remote: remote.is_valid(i).then(|| remote.value(i).to_string()),
                bytes: bytes.value(i),
                local_bytes: local_bytes.value(i),
                allocated_bytes: allocated_bytes.value(i),
                growth_bytes: growth_bytes.is_valid(i).then(|| growth_bytes.value(i)),
                regrowth_count: regrowth_count.value(i),
                worktree_count: worktree_count.value(i),
                observed_at: observed_at.value(i),
            });
        }
    }
    Ok(rows)
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StoredWorktreeRow {
    pub(crate) scope_key: String,
    pub(crate) worktree_id: String,
    pub(crate) project_id: String,
    pub(crate) path: String,
    /// `WorktreeKind`'s Debug label (`"Main"`/`"Linked"`/`"Clone"`).
    pub(crate) kind: String,
    pub(crate) branch: Option<String>,
    pub(crate) idle_secs: Option<u64>,
    pub(crate) github_default_branch: Option<String>,
    pub(crate) github_branch_exists_on_remote: Option<bool>,
    pub(crate) github_unavailable_reason: Option<String>,
    /// `MergedStatus`'s label (`"yes"`/`"no"`/`"unknown"`), only when
    /// `github` is `Some`.
    pub(crate) github_merged_state: Option<String>,
    pub(crate) github_merged_at: Option<String>,
    pub(crate) github_merged_pr_number: Option<u64>,
    /// `PrStatus`'s label (`"some"`/`"none"`/`"unknown"`).
    pub(crate) github_pr_state: Option<String>,
    pub(crate) github_pr_number: Option<u64>,
    /// `PrState`'s label (`"open"`/`"closed"`/`"merged"`), only when
    /// `pr_state == "some"`.
    pub(crate) github_pr_status: Option<String>,
    pub(crate) github_pr_draft: Option<bool>,
    pub(crate) github_pr_url: Option<String>,
    pub(crate) github_pr_title: Option<String>,
    pub(crate) github_pr_review_decision: Option<String>,
    pub(crate) github_pr_updated_at: Option<String>,
    /// `TriState`'s label (`"yes"`/`"no"`/`"unknown"`), only when
    /// `merge_complete` is `Some`.
    pub(crate) merge_complete_verdict: Option<String>,
    pub(crate) observed_at: u64,
}

fn worktrees_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("scope_key", DataType::Utf8, false),
        Field::new("worktree_id", DataType::Utf8, false),
        Field::new("project_id", DataType::Utf8, false),
        Field::new("path", DataType::Utf8, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new("branch", DataType::Utf8, true),
        Field::new("idle_secs", DataType::UInt64, true),
        Field::new("github_default_branch", DataType::Utf8, true),
        Field::new("github_branch_exists_on_remote", DataType::Boolean, true),
        Field::new("github_unavailable_reason", DataType::Utf8, true),
        Field::new("github_merged_state", DataType::Utf8, true),
        Field::new("github_merged_at", DataType::Utf8, true),
        Field::new("github_merged_pr_number", DataType::UInt64, true),
        Field::new("github_pr_state", DataType::Utf8, true),
        Field::new("github_pr_number", DataType::UInt64, true),
        Field::new("github_pr_status", DataType::Utf8, true),
        Field::new("github_pr_draft", DataType::Boolean, true),
        Field::new("github_pr_url", DataType::Utf8, true),
        Field::new("github_pr_title", DataType::Utf8, true),
        Field::new("github_pr_review_decision", DataType::Utf8, true),
        Field::new("github_pr_updated_at", DataType::Utf8, true),
        Field::new("merge_complete_verdict", DataType::Utf8, true),
        Field::new("observed_at", DataType::UInt64, false),
    ]))
}

macro_rules! opt_str_col {
    ($rows:expr, $field:ident) => {
        Arc::new(StringArray::from(
            $rows
                .iter()
                .map(|r| r.$field.as_deref())
                .collect::<Vec<Option<&str>>>(),
        )) as ArrayRef
    };
}

macro_rules! opt_u64_col {
    ($rows:expr, $field:ident) => {
        Arc::new(UInt64Array::from(
            $rows.iter().map(|r| r.$field).collect::<Vec<Option<u64>>>(),
        )) as ArrayRef
    };
}

macro_rules! opt_bool_col {
    ($rows:expr, $field:ident) => {
        Arc::new(BooleanArray::from(
            $rows
                .iter()
                .map(|r| r.$field)
                .collect::<Vec<Option<bool>>>(),
        )) as ArrayRef
    };
}

macro_rules! opt_i64_col {
    ($rows:expr, $field:ident) => {
        Arc::new(Int64Array::from(
            $rows.iter().map(|r| r.$field).collect::<Vec<Option<i64>>>(),
        )) as ArrayRef
    };
}

macro_rules! opt_f64_col {
    ($rows:expr, $field:ident) => {
        Arc::new(Float64Array::from(
            $rows.iter().map(|r| r.$field).collect::<Vec<Option<f64>>>(),
        )) as ArrayRef
    };
}

pub(crate) fn write_worktree_rows(path: &Path, rows: &[StoredWorktreeRow]) -> Result<()> {
    let schema = worktrees_schema();
    let scope_key: Vec<&str> = rows.iter().map(|r| r.scope_key.as_str()).collect();
    let worktree_id: Vec<&str> = rows.iter().map(|r| r.worktree_id.as_str()).collect();
    let project_id: Vec<&str> = rows.iter().map(|r| r.project_id.as_str()).collect();
    let path_col: Vec<&str> = rows.iter().map(|r| r.path.as_str()).collect();
    let kind: Vec<&str> = rows.iter().map(|r| r.kind.as_str()).collect();
    let observed_at: Vec<u64> = rows.iter().map(|r| r.observed_at).collect();

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(scope_key)) as ArrayRef,
            Arc::new(StringArray::from(worktree_id)),
            Arc::new(StringArray::from(project_id)),
            Arc::new(StringArray::from(path_col)),
            Arc::new(StringArray::from(kind)),
            opt_str_col!(rows, branch),
            opt_u64_col!(rows, idle_secs),
            opt_str_col!(rows, github_default_branch),
            opt_bool_col!(rows, github_branch_exists_on_remote),
            opt_str_col!(rows, github_unavailable_reason),
            opt_str_col!(rows, github_merged_state),
            opt_str_col!(rows, github_merged_at),
            opt_u64_col!(rows, github_merged_pr_number),
            opt_str_col!(rows, github_pr_state),
            opt_u64_col!(rows, github_pr_number),
            opt_str_col!(rows, github_pr_status),
            opt_bool_col!(rows, github_pr_draft),
            opt_str_col!(rows, github_pr_url),
            opt_str_col!(rows, github_pr_title),
            opt_str_col!(rows, github_pr_review_decision),
            opt_str_col!(rows, github_pr_updated_at),
            opt_str_col!(rows, merge_complete_verdict),
            Arc::new(UInt64Array::from(observed_at)),
        ],
    )?;
    crate::fs_gate::columns::write_parquet_atomic(
        path,
        schema,
        std::iter::once(Ok(batch)),
        super::ARTIFACT_ZSTD_LEVEL,
    )
}

fn opt_str(batch: &RecordBatch, name: &str, i: usize) -> Result<Option<String>> {
    let col = batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<StringArray>())
        .with_context(|| format!("column {name} is not Utf8"))?;
    Ok(col.is_valid(i).then(|| col.value(i).to_string()))
}

fn opt_u64(batch: &RecordBatch, name: &str, i: usize) -> Result<Option<u64>> {
    let col = batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<UInt64Array>())
        .with_context(|| format!("column {name} is not UInt64"))?;
    Ok(col.is_valid(i).then(|| col.value(i)))
}

fn opt_bool(batch: &RecordBatch, name: &str, i: usize) -> Result<Option<bool>> {
    let col = batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<BooleanArray>())
        .with_context(|| format!("column {name} is not Boolean"))?;
    Ok(col.is_valid(i).then(|| col.value(i)))
}

fn opt_i64(batch: &RecordBatch, name: &str, i: usize) -> Result<Option<i64>> {
    let col = batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<Int64Array>())
        .with_context(|| format!("column {name} is not Int64"))?;
    Ok(col.is_valid(i).then(|| col.value(i)))
}

fn opt_f64(batch: &RecordBatch, name: &str, i: usize) -> Result<Option<f64>> {
    let col = batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<Float64Array>())
        .with_context(|| format!("column {name} is not Float64"))?;
    Ok(col.is_valid(i).then(|| col.value(i)))
}

pub(crate) fn read_worktree_rows(path: &Path) -> Result<Vec<StoredWorktreeRow>> {
    let Some(reader) = crate::fs_gate::columns::open_parquet(path).with_context(|| {
        format!(
            "read {} (delete it; the next `swamp observe` rebuilds it)",
            path.display()
        )
    })?
    else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch?;
        let scope_key = downcast_str(&batch, "scope_key")?;
        let worktree_id = downcast_str(&batch, "worktree_id")?;
        let project_id = downcast_str(&batch, "project_id")?;
        let path_col = downcast_str(&batch, "path")?;
        let kind = downcast_str(&batch, "kind")?;
        let observed_at = downcast_u64(&batch, "observed_at")?;
        for i in 0..batch.num_rows() {
            rows.push(StoredWorktreeRow {
                scope_key: scope_key.value(i).to_string(),
                worktree_id: worktree_id.value(i).to_string(),
                project_id: project_id.value(i).to_string(),
                path: path_col.value(i).to_string(),
                kind: kind.value(i).to_string(),
                branch: opt_str(&batch, "branch", i)?,
                idle_secs: opt_u64(&batch, "idle_secs", i)?,
                github_default_branch: opt_str(&batch, "github_default_branch", i)?,
                github_branch_exists_on_remote: opt_bool(
                    &batch,
                    "github_branch_exists_on_remote",
                    i,
                )?,
                github_unavailable_reason: opt_str(&batch, "github_unavailable_reason", i)?,
                github_merged_state: opt_str(&batch, "github_merged_state", i)?,
                github_merged_at: opt_str(&batch, "github_merged_at", i)?,
                github_merged_pr_number: opt_u64(&batch, "github_merged_pr_number", i)?,
                github_pr_state: opt_str(&batch, "github_pr_state", i)?,
                github_pr_number: opt_u64(&batch, "github_pr_number", i)?,
                github_pr_status: opt_str(&batch, "github_pr_status", i)?,
                github_pr_draft: opt_bool(&batch, "github_pr_draft", i)?,
                github_pr_url: opt_str(&batch, "github_pr_url", i)?,
                github_pr_title: opt_str(&batch, "github_pr_title", i)?,
                github_pr_review_decision: opt_str(&batch, "github_pr_review_decision", i)?,
                github_pr_updated_at: opt_str(&batch, "github_pr_updated_at", i)?,
                merge_complete_verdict: opt_str(&batch, "merge_complete_verdict", i)?,
                observed_at: observed_at.value(i),
            });
        }
    }
    Ok(rows)
}

/// One list entry from a `WorktreeRow`'s `signals` or
/// `merge_complete.terms`, ordered by `seq` within `(worktree_id,
/// fact_kind)` so the original `Vec` order round-trips exactly.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StoredWorktreeFactRow {
    pub(crate) scope_key: String,
    pub(crate) worktree_id: String,
    /// `"signal"` or `"merge_complete_term"`.
    pub(crate) fact_kind: String,
    /// A signal's name; `None` for a `merge_complete_term` (a bare
    /// string list).
    pub(crate) name: Option<String>,
    pub(crate) value: String,
    pub(crate) seq: u32,
}

fn worktree_facts_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("scope_key", DataType::Utf8, false),
        Field::new("worktree_id", DataType::Utf8, false),
        Field::new("fact_kind", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, true),
        Field::new("value", DataType::Utf8, false),
        Field::new("seq", DataType::UInt32, false),
    ]))
}

pub(crate) fn write_worktree_fact_rows(path: &Path, rows: &[StoredWorktreeFactRow]) -> Result<()> {
    let schema = worktree_facts_schema();
    let scope_key: Vec<&str> = rows.iter().map(|r| r.scope_key.as_str()).collect();
    let worktree_id: Vec<&str> = rows.iter().map(|r| r.worktree_id.as_str()).collect();
    let fact_kind: Vec<&str> = rows.iter().map(|r| r.fact_kind.as_str()).collect();
    let name: Vec<Option<&str>> = rows.iter().map(|r| r.name.as_deref()).collect();
    let value: Vec<&str> = rows.iter().map(|r| r.value.as_str()).collect();
    let seq: Vec<u32> = rows.iter().map(|r| r.seq).collect();

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(scope_key)) as ArrayRef,
            Arc::new(StringArray::from(worktree_id)),
            Arc::new(StringArray::from(fact_kind)),
            Arc::new(StringArray::from(name)),
            Arc::new(StringArray::from(value)),
            Arc::new(UInt32Array::from(seq)),
        ],
    )?;
    crate::fs_gate::columns::write_parquet_atomic(
        path,
        schema,
        std::iter::once(Ok(batch)),
        super::ARTIFACT_ZSTD_LEVEL,
    )
}

pub(crate) fn read_worktree_fact_rows(path: &Path) -> Result<Vec<StoredWorktreeFactRow>> {
    let Some(reader) = crate::fs_gate::columns::open_parquet(path).with_context(|| {
        format!(
            "read {} (delete it; the next `swamp observe` rebuilds it)",
            path.display()
        )
    })?
    else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch?;
        let scope_key = downcast_str(&batch, "scope_key")?;
        let worktree_id = downcast_str(&batch, "worktree_id")?;
        let fact_kind = downcast_str(&batch, "fact_kind")?;
        let name = batch
            .column_by_name("name")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .context("column name is not Utf8")?;
        let value = downcast_str(&batch, "value")?;
        let seq = downcast_u32(&batch, "seq")?;
        for i in 0..batch.num_rows() {
            rows.push(StoredWorktreeFactRow {
                scope_key: scope_key.value(i).to_string(),
                worktree_id: worktree_id.value(i).to_string(),
                fact_kind: fact_kind.value(i).to_string(),
                name: name.is_valid(i).then(|| name.value(i).to_string()),
                value: value.value(i).to_string(),
                seq: seq.value(i),
            });
        }
    }
    Ok(rows)
}

// ---------------------------------------------------------------------
// external_units.parquet / agent_units.parquet (R16 item 1 of this
// slice; see `.oh/sessions/2026-09-24-r16-units-nested-evidence.md`).
// One shared row shape for both files (an `ExternalUnit` and an
// `AgentUnit` overlap enough that a second, near-identical struct would
// only be duplication): `id`/`source_id`/`source_name`/`category`/
// `path`/`bytes`/`mtime_max`/`observed_at`/`growth_bytes`/
// `regrowth_count` are common to both; `complete`/`linkage_state`/
// `linkage_basis`/`project_id`/`protected`/`protect_reason` are agent-
// only (an external unit has no partial-fold cap and no direct project
// link -- its associations are `unit_consumers.parquet`'s job) and left
// `None` for an external row. `consequence` is each unit's own `note`
// field (CHUNK_R16 names "consequence text"; neither struct has a field
// literally called that -- `note` is the closest fact each one carries,
// same "name what is actually there" choice R15 made for `remote`).
// `allocated` bytes: CHUNK_R16 lists it, but neither `ExternalUnit` nor
// `AgentUnit` has a bytes concept distinct from `bytes` itself (that
// distinction is `ArtifactRow`'s, already migrated in R15) -- there is
// nothing to store, so the column does not exist here; see the R16
// session note's "deviations" section.
//
// Not migrated (stay in the snapshot, overlaid by `id`): `provenance`
// (external), `tool_home`/`relative_path`/`members`/`action` (agent),
// `hardlinked`, `evidence` (moves to `evidence.parquet` instead of the
// overlay -- see below).
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StoredUnitRow {
    pub(crate) scope_key: String,
    pub(crate) id: String,
    pub(crate) source_id: String,
    pub(crate) source_name: String,
    pub(crate) category: String,
    pub(crate) path: String,
    pub(crate) bytes: u64,
    pub(crate) complete: Option<bool>,
    pub(crate) mtime_max: u64,
    /// `ProjectLinkState`'s label (`"linked"`/`"unresolved"`/
    /// `"missing"`/`"not-a-project"`/`"moved"`/`"remote"`), agent-only.
    pub(crate) linkage_state: Option<String>,
    /// The variant's own detail, flattened to one string (a reason, a
    /// path, a move's `from -> to`, a remote host); agent-only.
    pub(crate) linkage_basis: Option<String>,
    /// Only when `linkage_state == "linked"`.
    pub(crate) project_id: Option<String>,
    pub(crate) protected: Option<bool>,
    pub(crate) protect_reason: Option<String>,
    pub(crate) consequence: Option<String>,
    pub(crate) observed_at: u64,
    pub(crate) growth_bytes: Option<i64>,
    pub(crate) regrowth_count: u32,
}

fn unit_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("scope_key", DataType::Utf8, false),
        Field::new("id", DataType::Utf8, false),
        Field::new("source_id", DataType::Utf8, false),
        Field::new("source_name", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("path", DataType::Utf8, false),
        Field::new("bytes", DataType::UInt64, false),
        Field::new("complete", DataType::Boolean, true),
        Field::new("mtime_max", DataType::UInt64, false),
        Field::new("linkage_state", DataType::Utf8, true),
        Field::new("linkage_basis", DataType::Utf8, true),
        Field::new("project_id", DataType::Utf8, true),
        Field::new("protected", DataType::Boolean, true),
        Field::new("protect_reason", DataType::Utf8, true),
        Field::new("consequence", DataType::Utf8, true),
        Field::new("observed_at", DataType::UInt64, false),
        Field::new("growth_bytes", DataType::Int64, true),
        Field::new("regrowth_count", DataType::UInt32, false),
    ]))
}

pub(crate) fn write_unit_rows(path: &Path, rows: &[StoredUnitRow]) -> Result<()> {
    let schema = unit_schema();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.scope_key.as_str())
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.source_id.as_str())
                    .collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.source_name.as_str())
                    .collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.category.as_str()).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                rows.iter().map(|r| r.bytes).collect::<Vec<_>>(),
            )),
            opt_bool_col!(rows, complete),
            Arc::new(UInt64Array::from(
                rows.iter().map(|r| r.mtime_max).collect::<Vec<_>>(),
            )),
            opt_str_col!(rows, linkage_state),
            opt_str_col!(rows, linkage_basis),
            opt_str_col!(rows, project_id),
            opt_bool_col!(rows, protected),
            opt_str_col!(rows, protect_reason),
            opt_str_col!(rows, consequence),
            Arc::new(UInt64Array::from(
                rows.iter().map(|r| r.observed_at).collect::<Vec<_>>(),
            )),
            opt_i64_col!(rows, growth_bytes),
            Arc::new(UInt32Array::from(
                rows.iter().map(|r| r.regrowth_count).collect::<Vec<_>>(),
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

pub(crate) fn read_unit_rows(path: &Path) -> Result<Vec<StoredUnitRow>> {
    let Some(reader) = crate::fs_gate::columns::open_parquet(path).with_context(|| {
        format!(
            "read {} (delete it; the next `swamp observe` rebuilds it)",
            path.display()
        )
    })?
    else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch?;
        let scope_key = downcast_str(&batch, "scope_key")?;
        let id = downcast_str(&batch, "id")?;
        let source_id = downcast_str(&batch, "source_id")?;
        let source_name = downcast_str(&batch, "source_name")?;
        let category = downcast_str(&batch, "category")?;
        let path_col = downcast_str(&batch, "path")?;
        let bytes = downcast_u64(&batch, "bytes")?;
        let mtime_max = downcast_u64(&batch, "mtime_max")?;
        let observed_at = downcast_u64(&batch, "observed_at")?;
        let regrowth_count = downcast_u32(&batch, "regrowth_count")?;
        for i in 0..batch.num_rows() {
            rows.push(StoredUnitRow {
                scope_key: scope_key.value(i).to_string(),
                id: id.value(i).to_string(),
                source_id: source_id.value(i).to_string(),
                source_name: source_name.value(i).to_string(),
                category: category.value(i).to_string(),
                path: path_col.value(i).to_string(),
                bytes: bytes.value(i),
                complete: opt_bool(&batch, "complete", i)?,
                mtime_max: mtime_max.value(i),
                linkage_state: opt_str(&batch, "linkage_state", i)?,
                linkage_basis: opt_str(&batch, "linkage_basis", i)?,
                project_id: opt_str(&batch, "project_id", i)?,
                protected: opt_bool(&batch, "protected", i)?,
                protect_reason: opt_str(&batch, "protect_reason", i)?,
                consequence: opt_str(&batch, "consequence", i)?,
                observed_at: observed_at.value(i),
                growth_bytes: opt_i64(&batch, "growth_bytes", i)?,
                regrowth_count: regrowth_count.value(i),
            });
        }
    }
    Ok(rows)
}

/// `unit_consumers.parquet`: one row per `ExternalUnit::consumers`
/// entry (`AgentUnit` has no consumer list -- its single `project_link`
/// lives on the unit row itself, above). `ExternalConsumer` has no
/// `project_id`/`basis` field (only `label`/`note`); CHUNK_R16 names
/// `consumer_project_id` (always `None` -- nothing produces one yet)
/// and `basis` (`ExternalConsumer::note`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StoredUnitConsumerRow {
    pub(crate) scope_key: String,
    pub(crate) unit_id: String,
    pub(crate) consumer_label: String,
    pub(crate) consumer_project_id: Option<String>,
    pub(crate) basis: Option<String>,
    pub(crate) seq: u32,
}

fn unit_consumers_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("scope_key", DataType::Utf8, false),
        Field::new("unit_id", DataType::Utf8, false),
        Field::new("consumer_label", DataType::Utf8, false),
        Field::new("consumer_project_id", DataType::Utf8, true),
        Field::new("basis", DataType::Utf8, true),
        Field::new("seq", DataType::UInt32, false),
    ]))
}

pub(crate) fn write_unit_consumer_rows(path: &Path, rows: &[StoredUnitConsumerRow]) -> Result<()> {
    let schema = unit_consumers_schema();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.scope_key.as_str())
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.unit_id.as_str()).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.consumer_label.as_str())
                    .collect::<Vec<_>>(),
            )),
            opt_str_col!(rows, consumer_project_id),
            opt_str_col!(rows, basis),
            Arc::new(UInt32Array::from(
                rows.iter().map(|r| r.seq).collect::<Vec<_>>(),
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

pub(crate) fn read_unit_consumer_rows(path: &Path) -> Result<Vec<StoredUnitConsumerRow>> {
    let Some(reader) = crate::fs_gate::columns::open_parquet(path).with_context(|| {
        format!(
            "read {} (delete it; the next `swamp observe` rebuilds it)",
            path.display()
        )
    })?
    else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch?;
        let scope_key = downcast_str(&batch, "scope_key")?;
        let unit_id = downcast_str(&batch, "unit_id")?;
        let consumer_label = downcast_str(&batch, "consumer_label")?;
        let seq = downcast_u32(&batch, "seq")?;
        for i in 0..batch.num_rows() {
            rows.push(StoredUnitConsumerRow {
                scope_key: scope_key.value(i).to_string(),
                unit_id: unit_id.value(i).to_string(),
                consumer_label: consumer_label.value(i).to_string(),
                consumer_project_id: opt_str(&batch, "consumer_project_id", i)?,
                basis: opt_str(&batch, "basis", i)?,
                seq: seq.value(i),
            });
        }
    }
    Ok(rows)
}

// ---------------------------------------------------------------------
// nested_artifacts.parquet (R16 item 2 of this slice). One row per
// `NestedArtifact` -- both a per-project build-artifact interior
// (`Report.nested_artifacts`) and a shared-store interior
// (`ReportSnapshot.store_interiors`), which are the same production
// type from two different producers (`build_stores`/adapters vs.
// `external::discover_and_measure`); `origin` says which list a row
// came from so reading splits them back apart losslessly rather than
// guessing from `container_id`/path overlap. CHUNK_R16 names `id`,
// "parent artifact row key" (`container_id` -- the id of the *container*
// `NestedArtifact` whose own `path` equals the owning `ArtifactRow`'s
// path; there is no other "artifact row key" a `NestedArtifact` carries
// today), `adapter`, `family`, `role`, `path`, `bytes`, `basis`
// (`AccountingBasis`), `mtime` (nullable), `consequence`, and the four
// variant scalars. Everything else (`parent_id`, `membership`, `is_dir`,
// `device`, `inode`, `logical_bytes`, `physical_bytes`,
// `physical_total`, `coverage`, `producer_evidence`/`consumer_evidence`
// -- the older, narrower per-nested-unit evidence shape, distinct from
// `decision_evidence`/`crate::evidence::Evidence`, which moves to
// `evidence.parquet` instead -- `action_group`, `present`,
// `growth_bytes`, `regrowth_count`, `action`, `reported_by`,
// `writer_lock`, and the variant's `package`/`version`/`toolchain`/
// `features`/`generation`/`unknowns`) stays in the snapshot, overlaid by
// `id`, same as R15's artifact-render fields.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StoredNestedArtifactRow {
    pub(crate) scope_key: String,
    pub(crate) origin: String,
    pub(crate) id: String,
    pub(crate) container_id: Option<String>,
    pub(crate) adapter: Option<String>,
    pub(crate) family: String,
    pub(crate) role: String,
    pub(crate) path: String,
    pub(crate) bytes: u64,
    pub(crate) basis: String,
    pub(crate) mtime: Option<u64>,
    pub(crate) consequence: Option<String>,
    pub(crate) variant_profile: Option<String>,
    pub(crate) variant_configuration: Option<String>,
    pub(crate) variant_target: Option<String>,
    pub(crate) variant_arch: Option<String>,
}

fn nested_artifacts_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("scope_key", DataType::Utf8, false),
        Field::new("origin", DataType::Utf8, false),
        Field::new("id", DataType::Utf8, false),
        Field::new("container_id", DataType::Utf8, true),
        Field::new("adapter", DataType::Utf8, true),
        Field::new("family", DataType::Utf8, false),
        Field::new("role", DataType::Utf8, false),
        Field::new("path", DataType::Utf8, false),
        Field::new("bytes", DataType::UInt64, false),
        Field::new("basis", DataType::Utf8, false),
        Field::new("mtime", DataType::UInt64, true),
        Field::new("consequence", DataType::Utf8, true),
        Field::new("variant_profile", DataType::Utf8, true),
        Field::new("variant_configuration", DataType::Utf8, true),
        Field::new("variant_target", DataType::Utf8, true),
        Field::new("variant_arch", DataType::Utf8, true),
    ]))
}

pub(crate) fn write_nested_artifact_rows(
    path: &Path,
    rows: &[StoredNestedArtifactRow],
) -> Result<()> {
    let schema = nested_artifacts_schema();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.scope_key.as_str())
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.origin.as_str()).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            )),
            opt_str_col!(rows, container_id),
            opt_str_col!(rows, adapter),
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.family.as_str()).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.role.as_str()).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                rows.iter().map(|r| r.bytes).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.basis.as_str()).collect::<Vec<_>>(),
            )),
            opt_u64_col!(rows, mtime),
            opt_str_col!(rows, consequence),
            opt_str_col!(rows, variant_profile),
            opt_str_col!(rows, variant_configuration),
            opt_str_col!(rows, variant_target),
            opt_str_col!(rows, variant_arch),
        ],
    )?;
    crate::fs_gate::columns::write_parquet_atomic(
        path,
        schema,
        std::iter::once(Ok(batch)),
        super::ARTIFACT_ZSTD_LEVEL,
    )
}

pub(crate) fn read_nested_artifact_rows(path: &Path) -> Result<Vec<StoredNestedArtifactRow>> {
    let Some(reader) = crate::fs_gate::columns::open_parquet(path).with_context(|| {
        format!(
            "read {} (delete it; the next `swamp observe` rebuilds it)",
            path.display()
        )
    })?
    else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch?;
        let scope_key = downcast_str(&batch, "scope_key")?;
        let origin = downcast_str(&batch, "origin")?;
        let id = downcast_str(&batch, "id")?;
        let family = downcast_str(&batch, "family")?;
        let role = downcast_str(&batch, "role")?;
        let path_col = downcast_str(&batch, "path")?;
        let bytes = downcast_u64(&batch, "bytes")?;
        let basis = downcast_str(&batch, "basis")?;
        for i in 0..batch.num_rows() {
            rows.push(StoredNestedArtifactRow {
                scope_key: scope_key.value(i).to_string(),
                origin: origin.value(i).to_string(),
                id: id.value(i).to_string(),
                container_id: opt_str(&batch, "container_id", i)?,
                adapter: opt_str(&batch, "adapter", i)?,
                family: family.value(i).to_string(),
                role: role.value(i).to_string(),
                path: path_col.value(i).to_string(),
                bytes: bytes.value(i),
                basis: basis.value(i).to_string(),
                mtime: opt_u64(&batch, "mtime", i)?,
                consequence: opt_str(&batch, "consequence", i)?,
                variant_profile: opt_str(&batch, "variant_profile", i)?,
                variant_configuration: opt_str(&batch, "variant_configuration", i)?,
                variant_target: opt_str(&batch, "variant_target", i)?,
                variant_arch: opt_str(&batch, "variant_arch", i)?,
            });
        }
    }
    Ok(rows)
}

// ---------------------------------------------------------------------
// evidence.parquet (R16 item 3 of this slice). One row per
// `crate::evidence::Evidence` entry, attached to an `ArtifactRow`, an
// `ExternalUnit`, an `AgentUnit` or a `NestedArtifact`'s
// `decision_evidence` (the shared #53 contract; NOT `NestedArtifact`'s
// older, narrower `producer_evidence`/`consumer_evidence`, which stays
// in the snapshot). `row_key` is `"<entity_kind>:<entity_id>"`
// (`crate::growth::evidence_row_key`) so one table can key every entity
// kind without a join table per kind. `seq` keeps one entity's evidence
// `Vec` in order.
//
// CHUNK_R16's column list (`value_num`/`value_ts`/`value_text`) has no
// room for `FactValue`'s seven variants or for `FactStatus::Conflicting`
// (a `Vec<FactValue>`, always length >= 2 in production -- see
// `reclaimability.rs`/`toolchain_declarations.rs`/
// `external_associations.rs`); `value_kind` (the `FactValue` tag, needed
// to reconstruct e.g. `Bytes` vs `Count` from the same numeric column)
// and `conflicting_extra` (every candidate after the first, `|`-joined
// using each one's own textual form -- every production `Conflicting`
// site uses one `FactValue` variant across all its candidates, so one
// `value_kind` describes them all) are added for that reason, the same
// "extend when byte-identity needs it" call R15 made for `remote`.
// `reason` (`FactStatus::Unknown`/`Unavailable`/`Conflicting`'s message)
// and the two `freshness_*` columns are added for the same reason: named
// in `Evidence`/`FactStatus` but not in CHUNK_R16's list, and dropping
// them would change `agent_json`'s serialized `"evidence"` array.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StoredEvidenceRow {
    pub(crate) scope_key: String,
    pub(crate) row_key: String,
    pub(crate) seq: u32,
    pub(crate) kind: String,
    pub(crate) subtype: String,
    pub(crate) status: String,
    pub(crate) value_kind: Option<String>,
    pub(crate) value_num: Option<f64>,
    pub(crate) value_ts: Option<i64>,
    pub(crate) value_text: Option<String>,
    pub(crate) conflicting_extra: Option<String>,
    pub(crate) reason: Option<String>,
    pub(crate) source: String,
    pub(crate) source_detail: Option<String>,
    pub(crate) event_at: Option<i64>,
    pub(crate) observed_at: u64,
    pub(crate) freshness_expires_after_secs: Option<u64>,
    pub(crate) freshness_coverage_note: Option<String>,
    pub(crate) note: Option<String>,
}

fn evidence_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("scope_key", DataType::Utf8, false),
        Field::new("row_key", DataType::Utf8, false),
        Field::new("seq", DataType::UInt32, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new("subtype", DataType::Utf8, false),
        Field::new("status", DataType::Utf8, false),
        Field::new("value_kind", DataType::Utf8, true),
        Field::new("value_num", DataType::Float64, true),
        Field::new("value_ts", DataType::Int64, true),
        Field::new("value_text", DataType::Utf8, true),
        Field::new("conflicting_extra", DataType::Utf8, true),
        Field::new("reason", DataType::Utf8, true),
        Field::new("source", DataType::Utf8, false),
        Field::new("source_detail", DataType::Utf8, true),
        Field::new("event_at", DataType::Int64, true),
        Field::new("observed_at", DataType::UInt64, false),
        Field::new("freshness_expires_after_secs", DataType::UInt64, true),
        Field::new("freshness_coverage_note", DataType::Utf8, true),
        Field::new("note", DataType::Utf8, true),
    ]))
}

pub(crate) fn write_evidence_rows(path: &Path, rows: &[StoredEvidenceRow]) -> Result<()> {
    let schema = evidence_schema();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.scope_key.as_str())
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.row_key.as_str()).collect::<Vec<_>>(),
            )),
            Arc::new(UInt32Array::from(
                rows.iter().map(|r| r.seq).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.subtype.as_str()).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.status.as_str()).collect::<Vec<_>>(),
            )),
            opt_str_col!(rows, value_kind),
            opt_f64_col!(rows, value_num),
            opt_i64_col!(rows, value_ts),
            opt_str_col!(rows, value_text),
            opt_str_col!(rows, conflicting_extra),
            opt_str_col!(rows, reason),
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.source.as_str()).collect::<Vec<_>>(),
            )),
            opt_str_col!(rows, source_detail),
            opt_i64_col!(rows, event_at),
            Arc::new(UInt64Array::from(
                rows.iter().map(|r| r.observed_at).collect::<Vec<_>>(),
            )),
            opt_u64_col!(rows, freshness_expires_after_secs),
            opt_str_col!(rows, freshness_coverage_note),
            opt_str_col!(rows, note),
        ],
    )?;
    crate::fs_gate::columns::write_parquet_atomic(
        path,
        schema,
        std::iter::once(Ok(batch)),
        super::ARTIFACT_ZSTD_LEVEL,
    )
}

pub(crate) fn read_evidence_rows(path: &Path) -> Result<Vec<StoredEvidenceRow>> {
    let Some(reader) = crate::fs_gate::columns::open_parquet(path).with_context(|| {
        format!(
            "read {} (delete it; the next `swamp observe` rebuilds it)",
            path.display()
        )
    })?
    else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch?;
        let scope_key = downcast_str(&batch, "scope_key")?;
        let row_key = downcast_str(&batch, "row_key")?;
        let seq = downcast_u32(&batch, "seq")?;
        let kind = downcast_str(&batch, "kind")?;
        let subtype = downcast_str(&batch, "subtype")?;
        let status = downcast_str(&batch, "status")?;
        let source = downcast_str(&batch, "source")?;
        let observed_at = downcast_u64(&batch, "observed_at")?;
        for i in 0..batch.num_rows() {
            rows.push(StoredEvidenceRow {
                scope_key: scope_key.value(i).to_string(),
                row_key: row_key.value(i).to_string(),
                seq: seq.value(i),
                kind: kind.value(i).to_string(),
                subtype: subtype.value(i).to_string(),
                status: status.value(i).to_string(),
                value_kind: opt_str(&batch, "value_kind", i)?,
                value_num: opt_f64(&batch, "value_num", i)?,
                value_ts: opt_i64(&batch, "value_ts", i)?,
                value_text: opt_str(&batch, "value_text", i)?,
                conflicting_extra: opt_str(&batch, "conflicting_extra", i)?,
                reason: opt_str(&batch, "reason", i)?,
                source: source.value(i).to_string(),
                source_detail: opt_str(&batch, "source_detail", i)?,
                event_at: opt_i64(&batch, "event_at", i)?,
                observed_at: observed_at.value(i),
                freshness_expires_after_secs: opt_u64(&batch, "freshness_expires_after_secs", i)?,
                freshness_coverage_note: opt_str(&batch, "freshness_coverage_note", i)?,
                note: opt_str(&batch, "note", i)?,
            });
        }
    }
    Ok(rows)
}
