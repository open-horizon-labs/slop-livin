//! R4: growth since previous observation, from a reverse-delta store.
//!
//! Reuses the existing zstd/Parquet `Store` (see `store.rs`) rather than
//! adding a second persistence layer. Layout under
//! `${SLOP_LIVIN_DIR}/<volume-id>/`:
//!
//! - `current.parquet`: one row per known artifact key, holding its most
//!   recently observed value (including tombstones for keys that are no
//!   longer present, so a later reappearance can be counted as regrowth).
//! - `deltas/delta-<seq>.parquet`: one file per observation that changed
//!   at least one row. Each row in a delta file holds the *previous*
//!   value (before that observation) of a row that changed. Replaying
//!   deltas backwards from `current.parquet` reconstructs any past
//!   observation within the retention window.
//!
//! A row's identity is `(project_id, worktree_id, kind, rel_path)` where
//! `rel_path` is always relative to the worktree root -- never an
//! absolute path -- so the store stays portable across machines and
//! home-directory renames.
//!
//! Compaction: once the number of delta files crosses
//! [`COMPACTION_THRESHOLD`], every delta older than the retention window
//! is dropped and the remaining deltas are merged into a single file.
//! A no-change observation (no row's bytes/presence changed) appends no
//! delta file at all.

use crate::entities::Confidence;
use crate::fs_events::{FsEventsRequest, FsEventsState};
use crate::git::DiscoveredWorktree;
use crate::report::{
    ArtifactKind, ArtifactRow, DirRollup, FileRow, ProjectRow, Source, UnownedRow,
};
use anyhow::{Context, Result};
use arrow_array::{
    Array, ArrayRef, BooleanArray, Int32Array, RecordBatch, StringArray, UInt32Array, UInt64Array,
};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::{WriterProperties, WriterVersion};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const DEFAULT_RETENTION_DAYS: u64 = 30;
pub const DEFAULT_SINCE: &str = "24h";
/// R4c default threshold for a standalone large-file row: 1 MiB.
pub const DEFAULT_LARGE_FILE_MIN_BYTES: u64 = 1024 * 1024;
/// Default watchdog budget for one `observe` invocation.
pub const DEFAULT_OBSERVE_TIMEOUT_SEC: u64 = crate::schedule::DEFAULT_OBSERVE_TIMEOUT_SECS;
/// Delta files beyond this count trigger compaction into a single file.
const COMPACTION_THRESHOLD: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrowthConfig {
    pub retention_days: u64,
    pub since: String,
    pub large_file_min_bytes: u64,
    /// Watchdog budget for one `observe` invocation (item 3 of #31).
    pub observe_timeout_sec: u64,
}

impl Default for GrowthConfig {
    fn default() -> Self {
        Self {
            retention_days: DEFAULT_RETENTION_DAYS,
            since: DEFAULT_SINCE.to_string(),
            large_file_min_bytes: DEFAULT_LARGE_FILE_MIN_BYTES,
            observe_timeout_sec: DEFAULT_OBSERVE_TIMEOUT_SEC,
        }
    }
}

/// Reads `<slop_livin_dir>/config.toml` (`retention_days = 30`,
/// `since = "24h"`, `observe_timeout_sec = 1800`). A missing file, or keys
/// it does not recognize, fall back to defaults; this is a tiny
/// hand-rolled reader so the crate does not need a full TOML dependency
/// for a handful of scalar settings.
pub fn load_config(slop_livin_dir: &Path) -> GrowthConfig {
    let mut cfg = GrowthConfig::default();
    let Ok(text) = fs::read_to_string(slop_livin_dir.join("config.toml")) else {
        return cfg;
    };
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"').trim_matches('\'');
        match key {
            "retention_days" => {
                if let Ok(n) = value.parse() {
                    cfg.retention_days = n;
                }
            }
            "since" => cfg.since = value.to_string(),
            "large_file_min_bytes" => {
                if let Ok(n) = value.parse() {
                    cfg.large_file_min_bytes = n;
                }
            }
            "observe_timeout_sec" => {
                if let Ok(n) = value.parse() {
                    cfg.observe_timeout_sec = n;
                }
            }
            _ => {}
        }
    }
    cfg
}

/// Parses a duration like `"24h"`, `"30d"`, `"10m"`, `"45s"`, or a bare
/// number of seconds, into seconds.
pub fn parse_duration_secs(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(n) = s.parse::<u64>() {
        return Some(n);
    }
    let (num, unit) = s.split_at(s.len() - 1);
    let n: u64 = num.parse().ok()?;
    match unit {
        "s" => Some(n),
        "m" => Some(n * 60),
        "h" => Some(n * 3600),
        "d" => Some(n * 86400),
        _ => None,
    }
}

/// One row's storage identity: never an absolute path.
fn row_key(project_id: &str, worktree_id: &str, kind: &str, rel_path: &str) -> String {
    format!("{project_id}\u{1}{worktree_id}\u{1}{kind}\u{1}{rel_path}")
}

/// One stored row. Used both for `current.parquet` (where `bytes`/
/// `present` are the latest known value) and for delta files (where they
/// are the *previous* value, before the observation at `observed_at`).
#[derive(Debug, Clone)]
struct StoredRow {
    project_id: String,
    worktree_id: String,
    kind: String,
    rel_path: String,
    bytes: u64,
    local_bytes: u64,
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
        ],
    )?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = File::create(path).with_context(|| format!("create {}", path.display()))?;
    let properties = WriterProperties::builder()
        .set_compression(Compression::ZSTD(Default::default()))
        .set_writer_version(WriterVersion::PARQUET_2_0)
        .build();
    let mut writer = ArrowWriter::try_new(file, schema, Some(properties))?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}

fn read_rows(path: &Path) -> Result<Vec<StoredRow>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)?.build()?;
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
                    .filter(|v| *v != 0)
                    .unwrap_or_else(|| bytes.value(i)),
                present: present.value(i),
                observed_at: observed_at.value(i),
                regrowth_count: regrowth.value(i),
            });
        }
    }
    Ok(rows)
}

fn downcast_str<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a StringArray> {
    batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<StringArray>())
        .with_context(|| format!("column {name} is not Utf8"))
}
fn downcast_u64<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a UInt64Array> {
    batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<UInt64Array>())
        .with_context(|| format!("column {name} is not UInt64"))
}
fn downcast_u32<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a UInt32Array> {
    batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<UInt32Array>())
        .with_context(|| format!("column {name} is not UInt32"))
}
fn downcast_bool<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a BooleanArray> {
    batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<BooleanArray>())
        .with_context(|| format!("column {name} is not Boolean"))
}

fn volume_dir(slop_livin_dir: &Path, volume_id: u64) -> PathBuf {
    slop_livin_dir.join(volume_id.to_string())
}
fn current_path(dir: &Path) -> PathBuf {
    dir.join("current.parquet")
}
fn deltas_dir(dir: &Path) -> PathBuf {
    dir.join("deltas")
}

fn list_delta_files(dir: &Path) -> Vec<PathBuf> {
    let dir = deltas_dir(dir);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "parquet"))
        .collect();
    files.sort();
    files
}

fn next_delta_path(dir: &Path) -> PathBuf {
    let existing = list_delta_files(dir);
    let next_seq = existing
        .iter()
        .filter_map(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.strip_prefix("delta-"))
                .and_then(|s| s.parse::<u64>().ok())
        })
        .max()
        .map(|n| n + 1)
        .unwrap_or(0);
    deltas_dir(dir).join(format!("delta-{next_seq:012}.parquet"))
}

/// One observation's artifact facts, flattened out of the report's
/// project/worktree tree, keyed by row identity.
struct Observed {
    key: String,
    project_id: String,
    worktree_id: String,
    kind: String,
    rel_path: String,
    bytes: u64,
    local_bytes: u64,
}

fn flatten(projects: &[ProjectRow]) -> Vec<Observed> {
    let mut out = Vec::new();
    for project in projects {
        for worktree in &project.worktrees {
            for artifact in &worktree.artifacts {
                let rel_path = artifact
                    .path
                    .strip_prefix(&worktree.path)
                    .unwrap_or(&artifact.path)
                    .to_path_buf();
                let kind = format!("{:?}", artifact.kind);
                let rel_path_str = rel_path.display().to_string();
                out.push(Observed {
                    key: row_key(
                        &project.project_id,
                        &worktree.worktree_id,
                        &kind,
                        &rel_path_str,
                    ),
                    project_id: project.project_id.clone(),
                    worktree_id: worktree.worktree_id.clone(),
                    kind,
                    rel_path: rel_path_str,
                    bytes: artifact.bytes,
                    local_bytes: if artifact.local_bytes == 0 {
                        artifact.bytes
                    } else {
                        artifact.local_bytes
                    },
                });
            }
        }
    }
    out
}

/// Read-only counterpart to [`observe_and_annotate`]: annotates each
/// artifact row in `projects` with `growth_bytes`/`regrowth_count` from
/// whatever history the store already has, without writing a new
/// observation (no `current.parquet` update, no delta file). This is
/// what `--no-observe` and any other read-only report call use: growth
/// is a property of the store's existing observations, not of whether
/// *this* call is the one adding a new one. A row with no prior
/// observation in the store keeps `growth_bytes: None`, same as the
/// first-ever `observe_and_annotate` call would leave it.
pub fn annotate_readonly(
    slop_livin_dir: &Path,
    volume_id: u64,
    projects: &mut [ProjectRow],
    observed_at: u64,
    retention_days: u64,
    since_secs: u64,
) -> Result<()> {
    let dir = volume_dir(slop_livin_dir, volume_id);
    let current_file = current_path(&dir);
    if !current_file.exists() {
        // Store has never been observed for this volume; nothing to
        // annotate from.
        return Ok(());
    }
    let current_rows = read_rows(&current_file)?;
    let current_by_key: HashMap<String, &StoredRow> = current_rows
        .iter()
        .map(|r| {
            (
                row_key(&r.project_id, &r.worktree_id, &r.kind, &r.rel_path),
                r,
            )
        })
        .collect();

    let target_time = observed_at.saturating_sub(since_secs);
    for project in projects.iter_mut() {
        for worktree in project.worktrees.iter_mut() {
            for artifact in worktree.artifacts.iter_mut() {
                let rel_path = artifact
                    .path
                    .strip_prefix(&worktree.path)
                    .unwrap_or(&artifact.path)
                    .to_path_buf();
                let kind = format!("{:?}", artifact.kind);
                let key = row_key(
                    &project.project_id,
                    &worktree.worktree_id,
                    &kind,
                    &rel_path.display().to_string(),
                );
                let history = load_history(&dir, &key, retention_days, observed_at)?;
                artifact.growth_bytes = growth_since(&history, artifact.bytes, target_time);
                artifact.regrowth_count = current_by_key
                    .get(&key)
                    .map(|r| r.regrowth_count)
                    .unwrap_or(0);
            }
        }
    }
    Ok(())
}

/// Persists this observation's rows (current-state + delta) and
/// annotates each artifact row in `projects` with `growth_bytes` (since
/// `since_secs` ago) and `regrowth_count`.
///
/// `slop_livin_dir` is the top-level store root (e.g.
/// `${SLOP_LIVIN_DIR}`); the volume-keyed subdirectory is derived from
/// `volume_id`.
pub fn observe_and_annotate(
    slop_livin_dir: &Path,
    volume_id: u64,
    projects: &mut [ProjectRow],
    observed_at: u64,
    retention_days: u64,
    since_secs: u64,
) -> Result<()> {
    let dir = volume_dir(slop_livin_dir, volume_id);
    fs::create_dir_all(&dir)?;
    let current_file = current_path(&dir);

    let mut current: HashMap<String, StoredRow> = read_rows(&current_file)?
        .into_iter()
        .map(|r| {
            (
                row_key(&r.project_id, &r.worktree_id, &r.kind, &r.rel_path),
                r,
            )
        })
        .collect();

    let observed = flatten(projects);
    let mut seen_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut delta_rows: Vec<StoredRow> = Vec::new();

    for obs in &observed {
        seen_keys.insert(obs.key.clone());
        match current.get_mut(&obs.key) {
            Some(prev) => {
                let changed = prev.bytes != obs.bytes || !prev.present;
                if changed {
                    let regrowth_count = if !prev.present {
                        prev.regrowth_count + 1
                    } else {
                        prev.regrowth_count
                    };
                    // The delta's timestamp must be when this *old* value
                    // was itself last confirmed (`prev.observed_at`), not
                    // this observation's timestamp. Tagging it with the
                    // current observation instead collides with the new
                    // `current` row's own timestamp (both would read as
                    // "true at the same instant"), which makes the two
                    // conflicting values tie in `growth_since`'s
                    // nearest-timestamp lookup and lets an arbitrary one
                    // win.
                    delta_rows.push(StoredRow {
                        project_id: prev.project_id.clone(),
                        worktree_id: prev.worktree_id.clone(),
                        kind: prev.kind.clone(),
                        rel_path: prev.rel_path.clone(),
                        bytes: prev.bytes,
                        local_bytes: prev.local_bytes,
                        present: prev.present,
                        observed_at: prev.observed_at,
                        regrowth_count: prev.regrowth_count,
                    });
                    prev.bytes = obs.bytes;
                    prev.local_bytes = obs.local_bytes;
                    prev.present = true;
                    prev.observed_at = observed_at;
                    prev.regrowth_count = regrowth_count;
                }
            }
            None => {
                // Newly discovered row: there is no prior observation to
                // diff against, so this is not a "change" the delta log
                // needs to record -- it is simply the first known value.
                // Writing a synthetic "previously absent" delta here would
                // plant a fabricated (bytes=0, present=false) history
                // point at this observation's timestamp, which can tie
                // with (or beat) a real historical value once the row
                // later changes, corrupting `growth_since` lookups.
                current.insert(
                    obs.key.clone(),
                    StoredRow {
                        project_id: obs.project_id.clone(),
                        worktree_id: obs.worktree_id.clone(),
                        kind: obs.kind.clone(),
                        rel_path: obs.rel_path.clone(),
                        bytes: obs.bytes,
                        local_bytes: obs.local_bytes,
                        present: true,
                        observed_at,
                        regrowth_count: 0,
                    },
                );
            }
        }
    }

    // Rows present before, absent now: tombstone them (kept in the
    // store so a later reappearance counts as regrowth), but never
    // emitted as report rows in this issue.
    for (key, row) in current.iter_mut() {
        if row.present && !seen_keys.contains(key) {
            delta_rows.push(StoredRow {
                project_id: row.project_id.clone(),
                worktree_id: row.worktree_id.clone(),
                kind: row.kind.clone(),
                rel_path: row.rel_path.clone(),
                bytes: row.bytes,
                local_bytes: row.local_bytes,
                present: row.present,
                observed_at: row.observed_at,
                regrowth_count: row.regrowth_count,
            });
            row.present = false;
            row.bytes = 0;
            row.observed_at = observed_at;
        }
    }

    // Compute growth/regrowth for the artifacts in *this* report before
    // writing, using the pre-write history (current file on disk plus
    // any not-yet-written delta files already on disk).
    let target_time = observed_at.saturating_sub(since_secs);
    for project in projects.iter_mut() {
        for worktree in project.worktrees.iter_mut() {
            for artifact in worktree.artifacts.iter_mut() {
                let rel_path = artifact
                    .path
                    .strip_prefix(&worktree.path)
                    .unwrap_or(&artifact.path)
                    .to_path_buf();
                let kind = format!("{:?}", artifact.kind);
                let key = row_key(
                    &project.project_id,
                    &worktree.worktree_id,
                    &kind,
                    &rel_path.display().to_string(),
                );
                let history = load_history(&dir, &key, retention_days, observed_at)?;
                artifact.growth_bytes = growth_since(&history, artifact.bytes, target_time);
                artifact.regrowth_count = current.get(&key).map(|r| r.regrowth_count).unwrap_or(0);
            }
        }
    }

    if !delta_rows.is_empty() {
        let delta_path = next_delta_path(&dir);
        write_rows(&delta_path, &delta_rows)?;
    }

    let mut current_rows: Vec<StoredRow> = current.into_values().collect();
    current_rows.sort_by(|a, b| {
        (&a.project_id, &a.worktree_id, &a.kind, &a.rel_path).cmp(&(
            &b.project_id,
            &b.worktree_id,
            &b.kind,
            &b.rel_path,
        ))
    });
    write_rows(&current_file, &current_rows)?;

    compact_if_needed(&dir, retention_days, observed_at)?;

    Ok(())
}

/// One historical snapshot of a row's value: `(observed_at, bytes,
/// present)`, oldest first, ending with the value on disk right now
/// (before this observation's write).
fn load_history(
    dir: &Path,
    key: &str,
    retention_days: u64,
    now: u64,
) -> Result<Vec<(u64, u64, bool)>> {
    let retention_secs = retention_days.saturating_mul(86400);
    let horizon = now.saturating_sub(retention_secs);

    let current_rows = read_rows(&current_path(dir))?;
    let mut history: Vec<(u64, u64, bool)> = Vec::new();
    if let Some(row) = current_rows
        .iter()
        .find(|r| row_key(&r.project_id, &r.worktree_id, &r.kind, &r.rel_path) == key)
    {
        history.push((row.observed_at, row.bytes, row.present));
    }

    for delta_path in list_delta_files(dir) {
        for row in read_rows(&delta_path)? {
            if row_key(&row.project_id, &row.worktree_id, &row.kind, &row.rel_path) != key {
                continue;
            }
            if row.observed_at < horizon {
                continue;
            }
            history.push((row.observed_at, row.bytes, row.present));
        }
    }
    history.sort_by_key(|(t, _, _)| *t);
    Ok(history)
}

/// `growth_bytes` = bytes now minus bytes at the observation closest to
/// `target_time`, or `None` when no prior observation exists at all.
fn growth_since(history: &[(u64, u64, bool)], bytes_now: u64, target_time: u64) -> Option<i64> {
    if history.is_empty() {
        return None;
    }
    let closest = history
        .iter()
        .min_by_key(|(t, _, _)| t.abs_diff(target_time))?;
    Some(bytes_now as i64 - closest.1 as i64)
}

/// Merges every delta file into one, dropping deltas older than the
/// retention window, once the delta file count crosses
/// [`COMPACTION_THRESHOLD`].
fn compact_if_needed(dir: &Path, retention_days: u64, now: u64) -> Result<()> {
    let files = list_delta_files(dir);
    if files.len() <= COMPACTION_THRESHOLD {
        return Ok(());
    }
    let retention_secs = retention_days.saturating_mul(86400);
    let horizon = now.saturating_sub(retention_secs);

    let mut merged: Vec<StoredRow> = Vec::new();
    for path in &files {
        for row in read_rows(path)? {
            if row.observed_at >= horizon {
                merged.push(row);
            }
        }
    }
    for path in &files {
        fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
    }
    if !merged.is_empty() {
        merged.sort_by_key(|r| r.observed_at);
        write_rows(&next_delta_path(dir), &merged)?;
    }
    Ok(())
}

/// Prunes delta files that fall entirely outside the retention window,
/// without waiting for the compaction threshold. Exposed for callers
/// (or a future maintenance command) that want retention enforced on
/// every observation regardless of file count.
pub fn prune_expired(dir: &Path, retention_days: u64, now: u64) -> Result<()> {
    let retention_secs = retention_days.saturating_mul(86400);
    let horizon = now.saturating_sub(retention_secs);
    for path in list_delta_files(dir) {
        let rows = read_rows(&path)?;
        if rows.iter().all(|r| r.observed_at < horizon) {
            fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// R4c: dirs.parquet / files.parquet -- same current + reverse-delta
// layout as the artifact rows above, keyed by (worktree_id, rel_path)
// instead of (project_id, worktree_id, kind, rel_path). The current file
// is written zstd-9 (it is read on every observation and rewritten in
// full); delta files are written zstd-3 (cheap to append, most are
// pruned well before compaction).
// ---------------------------------------------------------------------

const DIR_BASE_ZSTD_LEVEL: i32 = 9;
const DIR_DELTA_ZSTD_LEVEL: i32 = 3;

fn list_files_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "parquet"))
        .collect();
    files.sort();
    files
}

fn next_seq_path(dir: &Path, prefix: &str) -> PathBuf {
    let existing = list_files_in(dir);
    let next_seq = existing
        .iter()
        .filter_map(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.strip_prefix(prefix))
                .and_then(|s| s.parse::<u64>().ok())
        })
        .max()
        .map(|n| n + 1)
        .unwrap_or(0);
    dir.join(format!("{prefix}{next_seq:012}.parquet"))
}

fn zstd_properties(level: i32) -> WriterProperties {
    let level = ZstdLevel::try_new(level).unwrap_or_default();
    WriterProperties::builder()
        .set_compression(Compression::ZSTD(level))
        .set_writer_version(WriterVersion::PARQUET_2_0)
        .build()
}

// --- dirs.parquet ---

#[derive(Debug, Clone)]
struct StoredDirRow {
    worktree_id: String,
    rel_path: String,
    parent_rel_path: Option<String>,
    allocated_total: u64,
    own_allocated: u64,
    file_count: u32,
    entry_count: u32,
    symlink_count: u32,
    mod_time_min: i32,
    complete: bool,
    observed_at: u64,
}

fn dir_row_key(worktree_id: &str, rel_path: &str) -> String {
    format!("{worktree_id}\u{1}{rel_path}")
}

fn dirs_current_path(dir: &Path) -> PathBuf {
    dir.join("dirs.parquet")
}
fn dirs_deltas_dir(dir: &Path) -> PathBuf {
    dir.join("dirs_deltas")
}

fn dirs_schema() -> Arc<Schema> {
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

fn write_dir_rows(path: &Path, rows: &[StoredDirRow], zstd_level: i32) -> Result<()> {
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
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = File::create(path).with_context(|| format!("create {}", path.display()))?;
    let mut writer = ArrowWriter::try_new(file, schema, Some(zstd_properties(zstd_level)))?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}

fn read_dir_rows(path: &Path) -> Result<Vec<StoredDirRow>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)?.build()?;
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

/// Builds a `key -> [(observed_at, value)]` index over every dir row's
/// history in one pass (current file + every delta file read exactly
/// once), instead of the naive per-row approach of re-reading every file
/// on disk for every single directory. On a tree with tens of thousands
/// of directories, re-reading is the difference between a few file reads
/// and tens of thousands: this index is what keeps a second observation
/// close to the first observation's wall time.
fn build_dir_history_index(
    dir: &Path,
    current: &HashMap<String, StoredDirRow>,
    retention_days: u64,
    now: u64,
) -> Result<HashMap<String, Vec<(u64, u64)>>> {
    let retention_secs = retention_days.saturating_mul(86400);
    let horizon = now.saturating_sub(retention_secs);
    let mut index: HashMap<String, Vec<(u64, u64)>> = HashMap::new();
    for (key, row) in current {
        index
            .entry(key.clone())
            .or_default()
            .push((row.observed_at, row.allocated_total));
    }
    for delta_path in list_files_in(&dirs_deltas_dir(dir)) {
        for row in read_dir_rows(&delta_path)? {
            if row.observed_at < horizon {
                continue;
            }
            let key = dir_row_key(&row.worktree_id, &row.rel_path);
            index
                .entry(key)
                .or_default()
                .push((row.observed_at, row.allocated_total));
        }
    }
    for values in index.values_mut() {
        values.sort_by_key(|(t, _)| *t);
    }
    Ok(index)
}

fn growth_since_u64(history: &[(u64, u64)], now_val: u64, target_time: u64) -> Option<i64> {
    let closest = history
        .iter()
        .min_by_key(|(t, _)| t.abs_diff(target_time))?;
    Some(now_val as i64 - closest.1 as i64)
}

/// Read-only counterpart to [`observe_and_annotate_dirs`], mirroring
/// [`annotate_readonly`] for artifacts: annotates `growth_bytes` from
/// whatever the store already has, without writing a new observation.
pub fn annotate_readonly_dirs(
    slop_livin_dir: &Path,
    volume_id: u64,
    dirs: &mut [crate::report::DirRollup],
    observed_at: u64,
    retention_days: u64,
    since_secs: u64,
) -> Result<()> {
    let dir = volume_dir(slop_livin_dir, volume_id);
    let current_file = dirs_current_path(&dir);
    if !current_file.exists() {
        return Ok(());
    }
    let current: HashMap<String, StoredDirRow> = read_dir_rows(&current_file)?
        .into_iter()
        .map(|r| (dir_row_key(&r.worktree_id, &r.rel_path), r))
        .collect();
    let history_index = build_dir_history_index(&dir, &current, retention_days, observed_at)?;
    let empty_history: Vec<(u64, u64)> = Vec::new();
    let target_time = observed_at.saturating_sub(since_secs);
    for row in dirs.iter_mut() {
        let key = dir_row_key(&row.worktree_id, &row.rel_path);
        let history = history_index.get(&key).unwrap_or(&empty_history);
        row.growth_bytes = growth_since_u64(history, row.allocated_total, target_time);
    }
    Ok(())
}

/// Persists this observation's dir rows (current + reverse-delta) and
/// annotates each `DirRollup` with `growth_bytes` (since `since_secs`
/// ago), the same shape as [`observe_and_annotate`] for artifacts. A
/// no-change row appends no delta, matching the artifact store's
/// contract.
pub fn observe_and_annotate_dirs(
    slop_livin_dir: &Path,
    volume_id: u64,
    dirs: &mut [crate::report::DirRollup],
    observed_at: u64,
    retention_days: u64,
    since_secs: u64,
) -> Result<()> {
    let dir = volume_dir(slop_livin_dir, volume_id);
    fs::create_dir_all(&dir)?;
    let current_file = dirs_current_path(&dir);

    let mut current: HashMap<String, StoredDirRow> = read_dir_rows(&current_file)?
        .into_iter()
        .map(|r| (dir_row_key(&r.worktree_id, &r.rel_path), r))
        .collect();

    let history_index = build_dir_history_index(&dir, &current, retention_days, observed_at)?;
    let empty_history: Vec<(u64, u64)> = Vec::new();

    let mut delta_rows: Vec<StoredDirRow> = Vec::new();
    let target_time = observed_at.saturating_sub(since_secs);

    for row in dirs.iter_mut() {
        let key = dir_row_key(&row.worktree_id, &row.rel_path);
        let history = history_index.get(&key).unwrap_or(&empty_history);
        row.growth_bytes = growth_since_u64(history, row.allocated_total, target_time);

        match current.get_mut(&key) {
            Some(prev) => {
                let changed = prev.allocated_total != row.allocated_total
                    || prev.own_allocated != row.own_allocated
                    || prev.file_count != row.file_count
                    || prev.entry_count != row.entry_count
                    || prev.symlink_count != row.symlink_count
                    || prev.mod_time_min != row.mod_time_min
                    || prev.complete != row.complete;
                if changed {
                    delta_rows.push(prev.clone());
                    prev.allocated_total = row.allocated_total;
                    prev.own_allocated = row.own_allocated;
                    prev.file_count = row.file_count;
                    prev.entry_count = row.entry_count;
                    prev.symlink_count = row.symlink_count;
                    prev.mod_time_min = row.mod_time_min;
                    prev.complete = row.complete;
                    prev.observed_at = observed_at;
                }
            }
            None => {
                current.insert(
                    key,
                    StoredDirRow {
                        worktree_id: row.worktree_id.clone(),
                        rel_path: row.rel_path.clone(),
                        parent_rel_path: row.parent_rel_path.clone(),
                        allocated_total: row.allocated_total,
                        own_allocated: row.own_allocated,
                        file_count: row.file_count,
                        entry_count: row.entry_count,
                        symlink_count: row.symlink_count,
                        mod_time_min: row.mod_time_min,
                        complete: row.complete,
                        observed_at,
                    },
                );
            }
        }
    }

    if !delta_rows.is_empty() {
        let delta_path = next_seq_path(&dirs_deltas_dir(&dir), "delta-");
        write_dir_rows(&delta_path, &delta_rows, DIR_DELTA_ZSTD_LEVEL)?;
    }

    let mut current_rows: Vec<StoredDirRow> = current.into_values().collect();
    current_rows.sort_by(|a, b| (&a.worktree_id, &a.rel_path).cmp(&(&b.worktree_id, &b.rel_path)));
    write_dir_rows(&current_file, &current_rows, DIR_BASE_ZSTD_LEVEL)?;

    compact_dir_deltas_if_needed(&dir, retention_days, observed_at)?;
    Ok(())
}

fn compact_dir_deltas_if_needed(dir: &Path, retention_days: u64, now: u64) -> Result<()> {
    let deltas_dir_path = dirs_deltas_dir(dir);
    let files = list_files_in(&deltas_dir_path);
    if files.len() <= COMPACTION_THRESHOLD {
        return Ok(());
    }
    let retention_secs = retention_days.saturating_mul(86400);
    let horizon = now.saturating_sub(retention_secs);
    let mut merged = Vec::new();
    for path in &files {
        for row in read_dir_rows(path)? {
            if row.observed_at >= horizon {
                merged.push(row);
            }
        }
    }
    for path in &files {
        fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
    }
    if !merged.is_empty() {
        merged.sort_by_key(|r| r.observed_at);
        write_dir_rows(
            &next_seq_path(&deltas_dir_path, "delta-"),
            &merged,
            DIR_DELTA_ZSTD_LEVEL,
        )?;
    }
    Ok(())
}

// --- files.parquet ---

#[derive(Debug, Clone)]
struct StoredFileRow {
    worktree_id: String,
    rel_path: String,
    allocated: u64,
    mod_time_min: i32,
    observed_at: u64,
}

fn file_row_key(worktree_id: &str, rel_path: &str) -> String {
    format!("{worktree_id}\u{1}{rel_path}")
}

fn files_current_path(dir: &Path) -> PathBuf {
    dir.join("files.parquet")
}
fn files_deltas_dir(dir: &Path) -> PathBuf {
    dir.join("files_deltas")
}

fn files_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("worktree_id", DataType::Utf8, false),
        Field::new("rel_path", DataType::Utf8, false),
        Field::new("allocated", DataType::UInt64, false),
        Field::new("mod_time_min", DataType::Int32, false),
        Field::new("observed_at", DataType::UInt64, false),
    ]))
}

fn write_file_rows(path: &Path, rows: &[StoredFileRow], zstd_level: i32) -> Result<()> {
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
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = File::create(path).with_context(|| format!("create {}", path.display()))?;
    let mut writer = ArrowWriter::try_new(file, schema, Some(zstd_properties(zstd_level)))?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}

fn read_file_rows(path: &Path) -> Result<Vec<StoredFileRow>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)?.build()?;
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

/// Same one-pass approach as [`build_dir_history_index`], for file rows.
fn build_file_history_index(
    dir: &Path,
    current: &HashMap<String, StoredFileRow>,
    retention_days: u64,
    now: u64,
) -> Result<HashMap<String, Vec<(u64, u64)>>> {
    let retention_secs = retention_days.saturating_mul(86400);
    let horizon = now.saturating_sub(retention_secs);
    let mut index: HashMap<String, Vec<(u64, u64)>> = HashMap::new();
    for (key, row) in current {
        index
            .entry(key.clone())
            .or_default()
            .push((row.observed_at, row.allocated));
    }
    for delta_path in list_files_in(&files_deltas_dir(dir)) {
        for row in read_file_rows(&delta_path)? {
            if row.observed_at < horizon {
                continue;
            }
            let key = file_row_key(&row.worktree_id, &row.rel_path);
            index
                .entry(key)
                .or_default()
                .push((row.observed_at, row.allocated));
        }
    }
    for values in index.values_mut() {
        values.sort_by_key(|(t, _)| *t);
    }
    Ok(index)
}

/// Read-only counterpart to [`observe_and_annotate_files`], mirroring
/// [`annotate_readonly_dirs`] for large-file rows.
pub fn annotate_readonly_files(
    slop_livin_dir: &Path,
    volume_id: u64,
    files: &mut [crate::report::FileRow],
    observed_at: u64,
    retention_days: u64,
    since_secs: u64,
) -> Result<()> {
    let dir = volume_dir(slop_livin_dir, volume_id);
    let current_file = files_current_path(&dir);
    if !current_file.exists() {
        return Ok(());
    }
    let current: HashMap<String, StoredFileRow> = read_file_rows(&current_file)?
        .into_iter()
        .map(|r| (file_row_key(&r.worktree_id, &r.rel_path), r))
        .collect();
    let history_index = build_file_history_index(&dir, &current, retention_days, observed_at)?;
    let empty_history: Vec<(u64, u64)> = Vec::new();
    let target_time = observed_at.saturating_sub(since_secs);
    for row in files.iter_mut() {
        let key = file_row_key(&row.worktree_id, &row.rel_path);
        let history = history_index.get(&key).unwrap_or(&empty_history);
        row.growth_bytes = growth_since_u64(history, row.allocated, target_time);
    }
    Ok(())
}

/// Same shape as [`observe_and_annotate_dirs`], for large-file rows.
pub fn observe_and_annotate_files(
    slop_livin_dir: &Path,
    volume_id: u64,
    files: &mut [crate::report::FileRow],
    observed_at: u64,
    retention_days: u64,
    since_secs: u64,
) -> Result<()> {
    let dir = volume_dir(slop_livin_dir, volume_id);
    fs::create_dir_all(&dir)?;
    let current_file = files_current_path(&dir);

    let mut current: HashMap<String, StoredFileRow> = read_file_rows(&current_file)?
        .into_iter()
        .map(|r| (file_row_key(&r.worktree_id, &r.rel_path), r))
        .collect();

    let history_index = build_file_history_index(&dir, &current, retention_days, observed_at)?;
    let empty_history: Vec<(u64, u64)> = Vec::new();

    let mut delta_rows: Vec<StoredFileRow> = Vec::new();
    let target_time = observed_at.saturating_sub(since_secs);

    for row in files.iter_mut() {
        let key = file_row_key(&row.worktree_id, &row.rel_path);
        let history = history_index.get(&key).unwrap_or(&empty_history);
        row.growth_bytes = growth_since_u64(history, row.allocated, target_time);

        match current.get_mut(&key) {
            Some(prev) => {
                let changed =
                    prev.allocated != row.allocated || prev.mod_time_min != row.mod_time_min;
                if changed {
                    delta_rows.push(prev.clone());
                    prev.allocated = row.allocated;
                    prev.mod_time_min = row.mod_time_min;
                    prev.observed_at = observed_at;
                }
            }
            None => {
                current.insert(
                    key,
                    StoredFileRow {
                        worktree_id: row.worktree_id.clone(),
                        rel_path: row.rel_path.clone(),
                        allocated: row.allocated,
                        mod_time_min: row.mod_time_min,
                        observed_at,
                    },
                );
            }
        }
    }

    if !delta_rows.is_empty() {
        let delta_path = next_seq_path(&files_deltas_dir(&dir), "delta-");
        write_file_rows(&delta_path, &delta_rows, DIR_DELTA_ZSTD_LEVEL)?;
    }

    let mut current_rows: Vec<StoredFileRow> = current.into_values().collect();
    current_rows.sort_by(|a, b| (&a.worktree_id, &a.rel_path).cmp(&(&b.worktree_id, &b.rel_path)));
    write_file_rows(&current_file, &current_rows, DIR_BASE_ZSTD_LEVEL)?;

    compact_file_deltas_if_needed(&dir, retention_days, observed_at)?;
    Ok(())
}

fn compact_file_deltas_if_needed(dir: &Path, retention_days: u64, now: u64) -> Result<()> {
    let deltas_dir_path = files_deltas_dir(dir);
    let files = list_files_in(&deltas_dir_path);
    if files.len() <= COMPACTION_THRESHOLD {
        return Ok(());
    }
    let retention_secs = retention_days.saturating_mul(86400);
    let horizon = now.saturating_sub(retention_secs);
    let mut merged = Vec::new();
    for path in &files {
        for row in read_file_rows(path)? {
            if row.observed_at >= horizon {
                merged.push(row);
            }
        }
    }
    for path in &files {
        fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
    }
    if !merged.is_empty() {
        merged.sort_by_key(|r| r.observed_at);
        write_file_rows(
            &next_seq_path(&deltas_dir_path, "delta-"),
            &merged,
            DIR_DELTA_ZSTD_LEVEL,
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------
// R4b: FSEvents-driven incremental observation.
//
// Two more sidecars live in the volume dir alongside the parquet files
// above, both small JSON, both rewritten in full on every observation:
//
// - `fsevents.json`: the last observed FSEvents event id + device, so
//   the next observation knows where to replay from (`fs_events.rs`).
// - `topology.json`: the discovered checkout/worktree list (path,
//   kind, project identity) as of the last observation. Artifact/dir/
//   file *bytes* already live in the parquet files above; this sidecar
//   is the structural piece (which worktrees exist, at which paths)
//   that the parquet rows alone cannot reconstruct, since `rel_path` is
//   always relative and never carries its worktree's root back.
//
// Together they let an incremental observation skip discovery and
// attribution entirely for everything FSEvents does not implicate,
// re-walking only what changed and carrying every other row forward
// with its previously observed value untouched.
// ---------------------------------------------------------------------

/// The structural half of one discovered checkout/worktree, persisted so
/// the next observation can carry it forward without re-running
/// discovery. Bytes are never stored here; those live in the parquet
/// current-state files, keyed by `worktree_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredWorktree {
    worktree_id: String,
    project_id: String,
    project_name: String,
    path: PathBuf,
    kind: crate::report::WorktreeKind,
    remote_url: Option<String>,
}

fn fsevents_state_path(dir: &Path) -> PathBuf {
    dir.join("fsevents.json")
}
fn topology_path(dir: &Path) -> PathBuf {
    dir.join("topology.json")
}
fn unowned_path(dir: &Path) -> PathBuf {
    dir.join("unowned.json")
}

fn read_fsevents_state(dir: &Path) -> FsEventsState {
    fs::read_to_string(fsevents_state_path(dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_fsevents_state(dir: &Path, state: &FsEventsState) -> Result<()> {
    fs::create_dir_all(dir)?;
    fs::write(fsevents_state_path(dir), serde_json::to_string(state)?)
        .with_context(|| format!("write {}", fsevents_state_path(dir).display()))
}

fn read_topology(dir: &Path) -> Option<Vec<StoredWorktree>> {
    let text = fs::read_to_string(topology_path(dir)).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_topology(dir: &Path, worktrees: &[StoredWorktree]) -> Result<()> {
    fs::create_dir_all(dir)?;
    fs::write(topology_path(dir), serde_json::to_string(worktrees)?)
        .with_context(|| format!("write {}", topology_path(dir).display()))
}

fn read_unowned(dir: &Path) -> Vec<UnownedRow> {
    fs::read_to_string(unowned_path(dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_unowned(dir: &Path, unowned: &[UnownedRow]) -> Result<()> {
    fs::create_dir_all(dir)?;
    fs::write(unowned_path(dir), serde_json::to_string(unowned)?)
        .with_context(|| format!("write {}", unowned_path(dir).display()))
}

fn parse_artifact_kind(s: &str) -> ArtifactKind {
    match s {
        "BuildOutput" => ArtifactKind::BuildOutput,
        "DependencyTree" => ArtifactKind::DependencyTree,
        "Git" => ArtifactKind::Git,
        "Cache" => ArtifactKind::Cache,
        "Source" => ArtifactKind::Source,
        "DockerImage" => ArtifactKind::DockerImage,
        "DockerBuildCache" => ArtifactKind::DockerBuildCache,
        "DockerVolume" => ArtifactKind::DockerVolume,
        "Loose" => ArtifactKind::Loose,
        _ => ArtifactKind::Unknown,
    }
}

/// Reconstructs a full [`crate::attribution::AttributionResult`] from the
/// growth store's current-state files: every artifact/dir/file row this
/// volume has ever observed and is still present, plus the last-known
/// unowned rows carried forward verbatim (unowned rows are only
/// refreshed by a full walk; see module docs).
///
/// Every `ArtifactRow::path` here is still **relative** (the raw
/// `rel_path` from storage); the caller re-joins it against each
/// worktree's root once the topology is loaded, since this function has
/// no access to worktree roots on its own.
fn reconstruct_attribution(dir: &Path) -> Result<crate::attribution::AttributionResult> {
    let current_rows = read_rows(&current_path(dir))?;
    let mut artifacts_by_worktree: HashMap<String, Vec<ArtifactRow>> = HashMap::new();
    let mut attributed_total = 0u64;
    // Docker rows are persisted for growth history but are NOT part of the
    // filesystem walk: the report re-derives them from daemon facts every
    // time and keeps them out of `walked_total`/`attributed`. Carrying
    // them into the reconstructed attribution both double-listed them and
    // inflated the incremental totals by their unique bytes (#29 live).
    let is_docker_kind = |k: &str| matches!(k, "DockerImage" | "DockerBuildCache" | "DockerVolume");
    for row in current_rows
        .iter()
        .filter(|r| r.present && !is_docker_kind(&r.kind))
    {
        attributed_total += row.bytes;
        artifacts_by_worktree
            .entry(row.worktree_id.clone())
            .or_default()
            .push(ArtifactRow {
                kind: parse_artifact_kind(&row.kind),
                path: PathBuf::from(&row.rel_path),
                bytes: row.bytes,
                local_bytes: row.local_bytes,
                growth_bytes: None,
                regrowth_count: row.regrowth_count,
                observed_at: row.observed_at,
                confidence: Confidence::High,
                source: Source::new("filesystem.walk"),
                note: None,
                created_at: None,
                containers: Vec::new(),
                shared_with: Vec::new(),
                dangling: false,
            });
    }

    let dirs: Vec<DirRollup> = read_dir_rows(&dirs_current_path(dir))?
        .into_iter()
        .map(|r| DirRollup {
            worktree_id: r.worktree_id,
            rel_path: r.rel_path,
            parent_rel_path: r.parent_rel_path,
            allocated_total: r.allocated_total,
            own_allocated: r.own_allocated,
            file_count: r.file_count,
            entry_count: r.entry_count,
            symlink_count: r.symlink_count,
            mod_time_min: r.mod_time_min,
            complete: r.complete,
            growth_bytes: None,
        })
        .collect();
    let files: Vec<FileRow> = read_file_rows(&files_current_path(dir))?
        .into_iter()
        .map(|r| FileRow {
            worktree_id: r.worktree_id,
            rel_path: r.rel_path,
            allocated: r.allocated,
            mod_time_min: r.mod_time_min,
            growth_bytes: None,
        })
        .collect();

    let unowned = read_unowned(dir);
    let unowned_total = unowned.iter().map(|u| u.bytes).sum();
    // `attributed_total` already sums every stored artifact row,
    // including each worktree's own `Source` row (whose bytes equal that
    // worktree's root `DirRollup.allocated_total`), so it is not summed a
    // second time from `dirs`.
    let walked_total = attributed_total + unowned_total;

    Ok(crate::attribution::AttributionResult {
        artifacts_by_worktree,
        unowned,
        walked_total,
        attributed_total,
        unowned_total,
        dirs,
        files,
    })
}

/// One tracked walk's outcome: the same `(discovered, attribution)` shape
/// [`crate::walk::discover_and_attribute`] returns, plus the mode/reason
/// a caller reports in the coverage block and the `observe` log line.
pub struct TrackedWalk {
    pub discovered: Vec<DiscoveredWorktree>,
    pub attribution: crate::attribution::AttributionResult,
    /// `"incremental"` or `"full"`.
    pub mode: &'static str,
    /// `"incremental"` on success; otherwise the refusal reason (see
    /// [`crate::fs_events::RefreshRefusal::as_str`]), or `"no_stored_event_id"`
    /// / `"full_forced"` for the two non-FSEvents reasons a walk is full.
    pub reason: &'static str,
    pub changed_dirs: usize,
}

/// Threshold past which re-walking piecemeal costs more than a full
/// walk: more than this fraction of previously known directories
/// implicated by one replay.
const TOO_MANY_CHANGES_FRACTION: f64 = 0.20;

/// The [`RefreshRefusal::TooSoon`] floor, in seconds. Overridable via
/// `SLOP_LIVIN_FSEVENTS_MIN_INTERVAL_SECS` so a test driving a canned
/// [`crate::fs_events::FsEventsSource`] -- which has no real FSEvents
/// log-persistence lag to protect against -- can set it to `0` and reach
/// the incremental path without a real `sleep`.
fn min_interval_secs() -> u64 {
    std::env::var("SLOP_LIVIN_FSEVENTS_MIN_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3)
}

/// Entry point for `report::report_full`: replaces a plain call to
/// `walk::discover_and_attribute` with one that tries FSEvents first and
/// falls back to a full walk on any refusal, `force_full`, or a missing
/// store. Always re-anchors the stored FSEvents id/topology/unowned
/// snapshot before returning, on both the incremental and full paths, so
/// the next call has a baseline to replay from regardless of which path
/// this one took.
pub fn observe_tracked(
    slop_livin_dir: &Path,
    root: &Path,
    observed_at: u64,
    large_file_min_bytes: u64,
    force_full: bool,
    observe: bool,
) -> Result<TrackedWalk> {
    observe_tracked_with_source(
        slop_livin_dir,
        root,
        observed_at,
        large_file_min_bytes,
        force_full,
        observe,
        crate::fs_events::platform_source().as_ref(),
    )
}

/// Same as [`observe_tracked`], with the [`crate::fs_events::FsEventsSource`]
/// supplied explicitly rather than resolved via [`crate::fs_events::platform_source`].
/// This is the seam integration tests use to exercise every refusal
/// reason and the incremental merge deterministically, with canned event
/// batches, so no test depends on the live `fseventsd`.
///
/// `observe` mirrors `growth::observe_and_annotate` vs `annotate_readonly`:
/// the walk itself (full or incremental) always happens, so the caller
/// gets a report of the tree's current state either way, but the
/// FSEvents/topology/unowned sidecars are only rewritten when `observe`
/// is true. A `--no-observe` read must not silently advance the stored
/// event id, or the next real observation would replay from a point it
/// never actually walked from.
#[allow(clippy::too_many_arguments)]
pub fn observe_tracked_with_source(
    slop_livin_dir: &Path,
    root: &Path,
    observed_at: u64,
    large_file_min_bytes: u64,
    force_full: bool,
    observe: bool,
    source: &dyn crate::fs_events::FsEventsSource,
) -> Result<TrackedWalk> {
    let volume_id = fs::metadata(root).map(|m| m.dev()).unwrap_or(0);
    let dir = volume_dir(slop_livin_dir, volume_id);
    fs::create_dir_all(&dir)?;

    // FSEvents always answers in canonical paths (ask about `/tmp/x` on
    // macOS and it replies about `/private/tmp/x`); everything else this
    // module stores or matches against (topology, artifact/dir/file
    // rows) is expressed in whatever form the caller's `root` already
    // was, unchanged from every walk before this feature existed. Rather
    // than canonicalize the whole walk (which would change every path
    // this crate has ever returned whenever the caller's root sits under
    // a symlink -- macOS's own default temp dir is exactly this shape),
    // [`rebase_from_canonical`] translates each `changed_dirs` entry back
    // into the caller's original root form immediately after the
    // replay, so every path downstream of this point stays in the one
    // form the rest of the crate already assumes.
    let prev_state = read_fsevents_state(&dir);

    // `force_full` (`--full`, and every pre-#29 caller: `report_full`,
    // `report_with*`, the MCP surface, every test that predates this
    // feature) must never touch the FSEvents source at all -- not the
    // real one (this crate runs alongside dozens of other concurrent
    // test/CLI processes on a shared machine, where `fseventsd` itself
    // can become the bottleneck under combined load; a `source.replay`
    // call that is merely slow under contention still burns wall time
    // this path has promised never to pay), and not even a canned one in
    // tests (there is nothing to answer). Skipping the call entirely,
    // rather than calling it and discarding the answer, is what actually
    // keeps this path load-free instead of just "load but ignore".
    if force_full {
        let result = full_walk(root, observed_at, large_file_min_bytes, "full_forced")?;
        if observe {
            write_topology(&dir, &to_stored_worktrees(&result.discovered))?;
            write_unowned(&dir, &result.attribution.unowned)?;
            // The stored FSEvents id/device is deliberately left as-is: a
            // forced full walk has nothing new to report there (no
            // replay ran), and an older stored id just means the next
            // real incremental attempt replays a larger, still-correct
            // window rather than a wrong one.
        }
        return Ok(result);
    }

    // FSEvents always answers in canonical paths (ask about `/tmp/x` on
    // macOS and it replies about `/private/tmp/x`); everything else this
    // module stores or matches against (topology, artifact/dir/file
    // rows) is expressed in whatever form the caller's `root` already
    // was, unchanged from every walk before this feature existed. Rather
    // than canonicalize the whole walk (which would change every path
    // this crate has ever returned whenever the caller's root sits under
    // a symlink -- macOS's own default temp dir is exactly this shape),
    // [`rebase_from_canonical`] translates each `changed_dirs` entry back
    // into the caller's original root form immediately after the
    // replay, so every path downstream of this point stays in the one
    // form the rest of the crate already assumes.
    let canonical_root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    // FSEvents' own persisted log can lag a write by longer than the
    // growth store's whole-second timestamp granularity, so a replay
    // requested this soon after the baseline cannot yet distinguish
    // "nothing changed" from "the change has not been logged yet" --
    // most visibly when two observations happen back-to-back (tests;
    // a scripted double-run), where a live FSEvents source can
    // legitimately report zero changes for a write that already
    // happened. Below this floor, skip straight to a full walk rather
    // than trust an answer FSEvents itself cannot yet vouch for.
    // Overridable via `SLOP_LIVIN_FSEVENTS_MIN_INTERVAL_SECS` so tests
    // that use a canned source (which has no real log-lag to protect
    // against) can set it to `0` and skip real sleeps entirely.
    let too_soon = prev_state
        .last_observed_at
        .is_some_and(|t| observed_at.saturating_sub(t) < min_interval_secs());
    let mut plan = source.replay(&FsEventsRequest {
        root: canonical_root.clone(),
        since: prev_state,
    });
    plan.changed_dirs = plan
        .changed_dirs
        .iter()
        .map(|p| rebase_from_canonical(&canonical_root, root, p))
        .collect();

    let prev_topology = read_topology(&dir);

    let result = if too_soon {
        full_walk(
            root,
            observed_at,
            large_file_min_bytes,
            crate::fs_events::RefreshRefusal::TooSoon.as_str(),
        )?
    } else if !plan.incremental {
        full_walk(root, observed_at, large_file_min_bytes, plan.reason_str())?
    } else {
        match prev_topology {
            None => full_walk(
                root,
                observed_at,
                large_file_min_bytes,
                "no_stored_event_id",
            )?,
            Some(ref topo) => {
                // Floored at a minimum so a tiny tree (a handful of
                // Source directories) doesn't trip the "too many
                // changes" guard on the very first touched file --
                // the guard exists to protect large trees, where a
                // fraction is the meaningful signal.
                let known_dirs = read_dir_rows(&dirs_current_path(&dir))?.len().max(20);
                if plan.changed_dirs.len() as f64 > TOO_MANY_CHANGES_FRACTION * known_dirs as f64 {
                    full_walk(root, observed_at, large_file_min_bytes, "too_many_changes")?
                } else {
                    apply_incremental(
                        topo,
                        &plan.changed_dirs,
                        observed_at,
                        large_file_min_bytes,
                        &dir,
                    )?
                }
            }
        }
    };

    if observe {
        // Re-anchor for the next call regardless of which path was taken.
        write_fsevents_state(
            &dir,
            &FsEventsState {
                event_id: Some(plan.current_event_id),
                device: plan.device,
                last_observed_at: Some(observed_at),
            },
        )?;
        write_topology(&dir, &to_stored_worktrees(&result.discovered))?;
        write_unowned(&dir, &result.attribution.unowned)?;
    }

    Ok(result)
}

/// Translates a canonical path (as FSEvents reports it) back into the
/// caller's original root form, so every path this module compares
/// against `changed_dirs` afterward is in the same non-canonical form
/// every other walk in this crate already uses. A path outside
/// `canonical_root` (should not happen -- the replay was scoped to that
/// root) is left as-is rather than dropped, matching this module's
/// general rule of keeping an uncertain path rather than silently
/// discarding it.
fn rebase_from_canonical(canonical_root: &Path, original_root: &Path, p: &Path) -> PathBuf {
    match p.strip_prefix(canonical_root) {
        Ok(rel) => original_root.join(rel),
        Err(_) => p.to_path_buf(),
    }
}

fn to_stored_worktrees(discovered: &[DiscoveredWorktree]) -> Vec<StoredWorktree> {
    discovered
        .iter()
        .map(|dw| StoredWorktree {
            worktree_id: crate::entities::id_for(&dw.path.display().to_string()),
            project_id: dw.project_id.clone(),
            project_name: dw.project_name.clone(),
            path: dw.path.clone(),
            kind: dw.kind.clone(),
            remote_url: dw.remote_url.clone(),
        })
        .collect()
}

fn full_walk(
    root: &Path,
    observed_at: u64,
    large_file_min_bytes: u64,
    reason: &'static str,
) -> Result<TrackedWalk> {
    let (discovered, attribution) =
        crate::walk::discover_and_attribute(root, observed_at, large_file_min_bytes)?;
    Ok(TrackedWalk {
        discovered,
        attribution,
        mode: "full",
        reason,
        changed_dirs: 0,
    })
}

/// The incremental path: re-walks only the worktrees/artifact roots
/// FSEvents implicated, carrying every other row forward from the store
/// unchanged (see [`reconstruct_attribution`]).
fn apply_incremental(
    prev: &[StoredWorktree],
    changed_dirs: &[PathBuf],
    observed_at: u64,
    large_file_min_bytes: u64,
    dir: &Path,
) -> Result<TrackedWalk> {
    let mut attribution = reconstruct_attribution(dir)?;
    let worktree_root: HashMap<String, PathBuf> = prev
        .iter()
        .map(|w| (w.worktree_id.clone(), w.path.clone()))
        .collect();
    // Rows reconstructed above carry a bare relative path; re-join it
    // against the worktree's root now that we know it.
    for (worktree_id, rows) in attribution.artifacts_by_worktree.iter_mut() {
        let Some(root) = worktree_root.get(worktree_id) else {
            continue;
        };
        for row in rows.iter_mut() {
            row.path = if row.path.as_os_str().is_empty() {
                root.clone()
            } else {
                root.join(&row.path)
            };
        }
    }

    let mut discovered: Vec<DiscoveredWorktree> = prev
        .iter()
        .map(|w| DiscoveredWorktree {
            project_id: w.project_id.clone(),
            project_name: w.project_name.clone(),
            path: w.path.clone(),
            kind: w.kind.clone(),
            remote_url: w.remote_url.clone(),
        })
        .collect();

    let mut worktrees_to_rewalk: HashSet<String> = HashSet::new();
    let mut artifact_roots_to_resize: HashMap<PathBuf, (String, ArtifactKind)> = HashMap::new();
    let mut discovery_scan_roots: Vec<PathBuf> = Vec::new();

    for changed in changed_dirs {
        let nearest = prev
            .iter()
            .filter(|w| changed.starts_with(&w.path))
            .max_by_key(|w| w.path.as_os_str().len());
        let Some(wt) = nearest else {
            // Outside every known worktree: might be a brand-new project
            // appearing under the root. Scan from here; if nothing is
            // found, this change is simply not reflected until the next
            // full walk (documented limitation of the incremental path).
            discovery_scan_roots.push(changed.clone());
            continue;
        };
        let artifact_hit = attribution
            .artifacts_by_worktree
            .get(&wt.worktree_id)
            .and_then(|rows| {
                rows.iter()
                    .filter(|r| {
                        r.kind != ArtifactKind::Source
                            && (changed == &r.path || changed.starts_with(&r.path))
                    })
                    .max_by_key(|r| r.path.as_os_str().len())
            });
        if let Some(row) = artifact_hit {
            artifact_roots_to_resize
                .insert(row.path.clone(), (wt.worktree_id.clone(), row.kind.clone()));
        } else {
            worktrees_to_rewalk.insert(wt.worktree_id.clone());
            // A changed directory that gained (or lost) a `.git` inside
            // an already-known worktree's tree is a nested checkout; the
            // worktree-level rewalk below re-sizes but does not itself
            // run project discovery, so scan explicitly too.
            if changed.join(".git").exists() {
                discovery_scan_roots.push(changed.clone());
            }
        }
    }

    // New checkouts/worktrees discovered under any scan root.
    for scan_root in &discovery_scan_roots {
        if let Ok(found) = crate::walk::discover_parallel(scan_root) {
            for dw in found {
                if discovered.iter().any(|w| w.path == dw.path) {
                    continue;
                }
                let worktree_id = crate::entities::id_for(&dw.path.display().to_string());
                worktrees_to_rewalk.insert(worktree_id.clone());
                discovered.push(dw);
            }
        }
    }
    // Rebuild the root lookup now that new worktrees may have been added.
    let worktree_root: HashMap<String, PathBuf> = discovered
        .iter()
        .map(|dw| {
            (
                crate::entities::id_for(&dw.path.display().to_string()),
                dw.path.clone(),
            )
        })
        .collect();

    // Drop worktrees whose root has disappeared entirely: their rows are
    // simply not carried into this result, which tombstones them the next
    // time `growth::observe_and_annotate*` runs (a present row this
    // observation no longer emits is marked absent automatically).
    discovered.retain(|dw| dw.path.exists());
    let discovered_ids: HashSet<String> = discovered
        .iter()
        .map(|dw| crate::entities::id_for(&dw.path.display().to_string()))
        .collect();
    attribution
        .artifacts_by_worktree
        .retain(|id, _| discovered_ids.contains(id));
    attribution
        .dirs
        .retain(|d| discovered_ids.contains(&d.worktree_id));
    attribution
        .files
        .retain(|f| discovered_ids.contains(&f.worktree_id));

    let mut total_delta: i64 = 0;

    // Resize individual artifact roots.
    for (root_path, (worktree_id, kind)) in &artifact_roots_to_resize {
        if worktrees_to_rewalk.contains(worktree_id) {
            continue; // superseded by the full worktree rewalk below.
        }
        if !root_path.exists() {
            // The artifact directory itself was removed: drop the row
            // entirely rather than leaving a phantom zero-byte entry a
            // full walk would never have produced. A later change that
            // recreates this path finds no artifact_hit for it next
            // time (the row is gone), so it correctly falls through to
            // a worktree rewalk instead of a resize.
            if let Some(rows) = attribution.artifacts_by_worktree.get_mut(worktree_id)
                && let Some(pos) = rows.iter().position(|r| &r.path == root_path)
            {
                total_delta -= rows.remove(pos).bytes as i64;
            }
            continue;
        }
        let new_row = crate::walk::resize_artifact(root_path, kind.clone(), observed_at);
        if let Some(rows) = attribution.artifacts_by_worktree.get_mut(worktree_id) {
            if let Some(existing) = rows.iter_mut().find(|r| &r.path == root_path) {
                // Hardlink-safe: the full walk charged shared inodes to
                // whichever row saw them first, so compare per-row local
                // figures and apply that delta to the globally-deduped
                // `bytes` instead of replacing it with a re-count (#29).
                let old_local = if existing.local_bytes == 0 {
                    existing.bytes
                } else {
                    existing.local_bytes
                };
                let delta = new_row.local_bytes as i64 - old_local as i64;
                let mut merged = new_row;
                merged.bytes = (existing.bytes as i64 + delta).max(0) as u64;
                total_delta += delta;
                *existing = merged;
            } else {
                total_delta += new_row.bytes as i64;
                rows.push(new_row);
            }
        }
    }

    // Rewalk whole worktrees whose Source tree (or newly discovered
    // subtree) was implicated. `all_worktree_refs` is the *complete*
    // known worktree list (every worktree, not just the one being
    // rewalked): a linked worktree frequently lives inside its main
    // checkout's own directory tree (e.g. `.worktrees/<name>`), so
    // walking with only one worktree in the known list would let
    // `nearest_worktree` fold a nested worktree's own bytes into this
    // one -- on top of that nested worktree's unrelated, still-correct
    // carried-forward rows, double counting them. Passing the full list
    // keeps nested-worktree boundaries exactly as a full walk would;
    // only the entries keyed by *this* `worktree_id` are taken out of
    // the result below, since every other worktree here (including any
    // nested one this walk happened to pass through) keeps its
    // carried-forward rows untouched.
    let worktree_ids: Vec<String> = discovered
        .iter()
        .map(|dw| crate::entities::id_for(&dw.path.display().to_string()))
        .collect();
    let all_worktree_refs: Vec<(&Path, &str)> = discovered
        .iter()
        .zip(worktree_ids.iter())
        .map(|(dw, id)| (dw.path.as_path(), id.as_str()))
        .collect();

    for worktree_id in &worktrees_to_rewalk {
        let Some(root) = worktree_root.get(worktree_id) else {
            continue;
        };
        let fresh = crate::walk::attribute_one_worktree(
            root,
            &all_worktree_refs,
            observed_at,
            large_file_min_bytes,
        );
        // Merge per row by (kind, path): a row present before and after
        // keeps its globally-deduped `bytes` adjusted by the change in its
        // own per-row local figure; a brand-new row starts from its local
        // figure; a vanished row is subtracted in full (#29 hardlinks).
        let old_rows = attribution
            .artifacts_by_worktree
            .remove(worktree_id)
            .unwrap_or_default();
        let mut merged: Vec<ArtifactRow> = Vec::new();
        let fresh_rows = fresh
            .artifacts_by_worktree
            .get(worktree_id)
            .cloned()
            .unwrap_or_default();
        let mut matched = vec![false; old_rows.len()];
        for mut nr in fresh_rows {
            let pos = old_rows
                .iter()
                .position(|o| o.kind == nr.kind && o.path == nr.path);
            match pos {
                Some(i) => {
                    matched[i] = true;
                    let o = &old_rows[i];
                    let old_local = if o.local_bytes == 0 {
                        o.bytes
                    } else {
                        o.local_bytes
                    };
                    let new_local = if nr.local_bytes == 0 {
                        nr.bytes
                    } else {
                        nr.local_bytes
                    };
                    let delta = new_local as i64 - old_local as i64;
                    nr.bytes = (o.bytes as i64 + delta).max(0) as u64;
                    nr.regrowth_count = o.regrowth_count;
                    total_delta += delta;
                }
                None => {
                    let local = if nr.local_bytes == 0 {
                        nr.bytes
                    } else {
                        nr.local_bytes
                    };
                    nr.bytes = local;
                    total_delta += local as i64;
                }
            }
            merged.push(nr);
        }
        for (i, o) in old_rows.iter().enumerate() {
            if !matched[i] {
                total_delta -= o.bytes as i64;
            }
        }
        if merged.is_empty() {
            attribution.artifacts_by_worktree.remove(worktree_id);
        } else {
            attribution
                .artifacts_by_worktree
                .insert(worktree_id.clone(), merged);
        }
        // Only this worktree's own dir/file rows come out of `fresh`;
        // any nested worktree's rows the walk happened to also produce
        // are discarded here (that worktree's carried-forward rows are
        // already correct and were not queued for rewalk).
        attribution.dirs.retain(|d| &d.worktree_id != worktree_id);
        attribution.dirs.extend(
            fresh
                .dirs
                .into_iter()
                .filter(|d| &d.worktree_id == worktree_id),
        );
        attribution.files.retain(|f| &f.worktree_id != worktree_id);
        attribution.files.extend(
            fresh
                .files
                .into_iter()
                .filter(|f| &f.worktree_id == worktree_id),
        );
    }

    attribution.attributed_total =
        (attribution.attributed_total as i64 + total_delta).max(0) as u64;
    attribution.walked_total = (attribution.walked_total as i64 + total_delta).max(0) as u64;

    Ok(TrackedWalk {
        discovered,
        attribution,
        mode: "incremental",
        reason: "incremental",
        changed_dirs: changed_dirs.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_durations() {
        assert_eq!(parse_duration_secs("24h"), Some(24 * 3600));
        assert_eq!(parse_duration_secs("30d"), Some(30 * 86400));
        assert_eq!(parse_duration_secs("10m"), Some(600));
        assert_eq!(parse_duration_secs("45s"), Some(45));
        assert_eq!(parse_duration_secs("90"), Some(90));
        assert_eq!(parse_duration_secs(""), None);
        assert_eq!(parse_duration_secs("bogus"), None);
    }

    #[test]
    fn config_defaults_when_file_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = load_config(tmp.path());
        assert_eq!(cfg.retention_days, DEFAULT_RETENTION_DAYS);
        assert_eq!(cfg.since, DEFAULT_SINCE);
    }

    #[test]
    fn config_reads_toml_scalars() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("config.toml"),
            "retention_days = 14\nsince = \"6h\"\n",
        )
        .unwrap();
        let cfg = load_config(tmp.path());
        assert_eq!(cfg.retention_days, 14);
        assert_eq!(cfg.since, "6h");
    }

    use crate::entities::Confidence;
    use crate::report::{ArtifactKind, ArtifactRow, ProjectRow, Source, WorktreeKind, WorktreeRow};
    use std::path::PathBuf;

    fn one_artifact_project(worktree_root: &Path, bytes: u64) -> ProjectRow {
        ProjectRow {
            project_id: "proj-1".to_string(),
            name: "proj".to_string(),
            remote: None,
            worktrees: vec![WorktreeRow {
                worktree_id: "wt-1".to_string(),
                path: worktree_root.to_path_buf(),
                kind: WorktreeKind::Main,
                artifacts: vec![ArtifactRow {
                    kind: ArtifactKind::DependencyTree,
                    path: worktree_root.join("node_modules"),
                    bytes,
                    local_bytes: 0,
                    growth_bytes: None,
                    regrowth_count: 0,
                    observed_at: 0,
                    confidence: Confidence::High,
                    source: Source::new("test"),
                    note: None,
                    created_at: None,
                    containers: Vec::new(),
                    shared_with: Vec::new(),
                    dangling: false,
                }],
                signals: vec![],
                branch: None,
                github: None,
                merge_complete: None,
                idle_secs: None,
            }],
        }
    }

    fn artifact_row(projects: &[ProjectRow]) -> &ArtifactRow {
        &projects[0].worktrees[0].artifacts[0]
    }

    #[test]
    fn first_observation_has_no_growth() {
        let tmp = tempfile::tempdir().unwrap();
        let root = PathBuf::from("/repo");
        let mut projects = vec![one_artifact_project(&root, 1_000_000)];
        observe_and_annotate(tmp.path(), 1, &mut projects, 1_000, 30, 3600).unwrap();
        assert_eq!(artifact_row(&projects).growth_bytes, None);
        assert_eq!(artifact_row(&projects).regrowth_count, 0);
    }

    #[test]
    fn second_observation_reports_growth_since_first() {
        let tmp = tempfile::tempdir().unwrap();
        let root = PathBuf::from("/repo");

        let mut first = vec![one_artifact_project(&root, 1_000_000)];
        observe_and_annotate(tmp.path(), 1, &mut first, 1_000, 30, 3600).unwrap();

        let mut second = vec![one_artifact_project(&root, 4_000_000)];
        observe_and_annotate(tmp.path(), 1, &mut second, 2_000, 30, 3600).unwrap();

        assert_eq!(artifact_row(&second).growth_bytes, Some(3_000_000));
    }

    #[test]
    fn unchanged_observation_appends_no_delta_file() {
        let tmp = tempfile::tempdir().unwrap();
        let root = PathBuf::from("/repo");

        let mut first = vec![one_artifact_project(&root, 1_000_000)];
        observe_and_annotate(tmp.path(), 1, &mut first, 1_000, 30, 3600).unwrap();
        let dir = volume_dir(tmp.path(), 1);
        let after_first = list_delta_files(&dir).len();
        assert_eq!(
            after_first, 0,
            "a brand-new row has no prior state to diff against, so the very \
             first observation must not fabricate a delta either"
        );

        let mut second = vec![one_artifact_project(&root, 1_000_000)];
        observe_and_annotate(tmp.path(), 1, &mut second, 2_000, 30, 3600).unwrap();
        let after_second = list_delta_files(&dir).len();
        assert_eq!(
            after_second, after_first,
            "no-change observation must not append a delta file"
        );
        assert_eq!(artifact_row(&second).growth_bytes, Some(0));
    }

    #[test]
    fn absence_then_reappearance_counts_one_regrowth() {
        let tmp = tempfile::tempdir().unwrap();
        let root = PathBuf::from("/repo");

        let mut present = vec![one_artifact_project(&root, 1_000_000)];
        observe_and_annotate(tmp.path(), 1, &mut present, 1_000, 30, 3600).unwrap();

        // target/ deleted: no artifacts observed this pass at all.
        let mut absent: Vec<ProjectRow> = vec![ProjectRow {
            project_id: "proj-1".to_string(),
            name: "proj".to_string(),
            remote: None,
            worktrees: vec![WorktreeRow {
                worktree_id: "wt-1".to_string(),
                path: root.clone(),
                kind: WorktreeKind::Main,
                artifacts: vec![],
                signals: vec![],
                branch: None,
                github: None,
                merge_complete: None,
                idle_secs: None,
            }],
        }];
        observe_and_annotate(tmp.path(), 1, &mut absent, 2_000, 30, 3600).unwrap();

        // target/ recreated.
        let mut recreated = vec![one_artifact_project(&root, 500_000)];
        observe_and_annotate(tmp.path(), 1, &mut recreated, 3_000, 30, 3600).unwrap();

        assert_eq!(artifact_row(&recreated).regrowth_count, 1);
    }

    /// Regression for the bug reported against the real `~/src` live run:
    /// grow a row, then shrink it back to its original size. Growth at
    /// the 3rd observation, measured against a baseline far enough back
    /// to predate the very first observation, must be 0 -- the row is
    /// back to the value it started at. Before the fix, the delta
    /// written for a *brand-new* row (a synthetic "previously absent,
    /// bytes=0" entry timestamped at the row's first observation) could
    /// tie with -- and be preferred over -- the real historical entry
    /// once the row changed twice, so this returned `bytes_now` (as if
    /// there were no prior observation at all) instead of `0`.
    #[test]
    fn growth_after_grow_then_shrink_back_to_original_is_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let root = PathBuf::from("/repo");
        let original_bytes = 1_000_000;

        let mut obs1 = vec![one_artifact_project(&root, original_bytes)];
        observe_and_annotate(tmp.path(), 1, &mut obs1, 1_000, 30, 5_000).unwrap();

        let mut obs2 = vec![one_artifact_project(&root, original_bytes + 200_000_000)];
        observe_and_annotate(tmp.path(), 1, &mut obs2, 2_000, 30, 5_000).unwrap();

        // Shrunk back to exactly the original size.
        let mut obs3 = vec![one_artifact_project(&root, original_bytes)];
        observe_and_annotate(tmp.path(), 1, &mut obs3, 3_000, 30, 5_000).unwrap();

        assert_eq!(
            artifact_row(&obs3).growth_bytes,
            Some(0),
            "back to the original size: growth against a far-back baseline must be 0, \
             not bytes_now as if no prior observation existed"
        );
    }

    /// After two consecutive changes, a growth query whose baseline time
    /// lands on the *first* change must use that change's real recorded
    /// value, never fall through to treating the row as previously
    /// absent (bytes=0).
    #[test]
    fn growth_after_two_changes_uses_correct_historical_value_not_absence() {
        let tmp = tempfile::tempdir().unwrap();
        let root = PathBuf::from("/repo");

        let mut obs1 = vec![one_artifact_project(&root, 1_000_000)];
        observe_and_annotate(tmp.path(), 1, &mut obs1, 1_000, 30, 2_000).unwrap();

        let mut obs2 = vec![one_artifact_project(&root, 2_000_000)];
        observe_and_annotate(tmp.path(), 1, &mut obs2, 2_000, 30, 2_000).unwrap();

        // since_secs=2_000 at observed_at=3_000 targets time 1_000 --
        // exactly obs1's timestamp -- so the baseline must be obs1's
        // 1_000_000 bytes, not 0.
        let mut obs3 = vec![one_artifact_project(&root, 5_000_000)];
        observe_and_annotate(tmp.path(), 1, &mut obs3, 3_000, 30, 2_000).unwrap();

        assert_eq!(
            artifact_row(&obs3).growth_bytes,
            Some(4_000_000),
            "baseline must be obs1's real recorded value (1_000_000), not absence (0)"
        );
    }

    #[test]
    fn delta_count_crossing_threshold_compacts() {
        let tmp = tempfile::tempdir().unwrap();
        let root = PathBuf::from("/repo");
        let dir = volume_dir(tmp.path(), 1);

        for i in 0..(COMPACTION_THRESHOLD as u64 + 5) {
            let mut obs = vec![one_artifact_project(&root, 1_000 + i)];
            observe_and_annotate(tmp.path(), 1, &mut obs, 1_000 + i, 30, 3600).unwrap();
        }

        let files = list_delta_files(&dir);
        assert!(
            files.len() <= COMPACTION_THRESHOLD,
            "expected compaction to keep the delta file count bounded, got {}",
            files.len()
        );
        assert!(
            !files.is_empty(),
            "compacted history must not be discarded entirely"
        );
    }

    #[test]
    fn row_keys_never_carry_an_absolute_path() {
        // The stored rel_path for an artifact under the worktree root must
        // be relative, never the artifact's absolute filesystem path.
        let tmp = tempfile::tempdir().unwrap();
        let root = PathBuf::from("/some/absolute/worktree/root");
        let mut projects = vec![one_artifact_project(&root, 42)];
        observe_and_annotate(tmp.path(), 1, &mut projects, 1_000, 30, 3600).unwrap();

        let dir = volume_dir(tmp.path(), 1);
        let current = read_rows(&current_path(&dir)).unwrap();
        let row = current
            .iter()
            .find(|r| r.rel_path.contains("node_modules"))
            .unwrap();
        assert_eq!(row.rel_path, "node_modules");
        assert!(!row.rel_path.starts_with('/'));
    }
}
