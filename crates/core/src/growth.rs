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

use crate::report::ProjectRow;
use anyhow::{Context, Result};
use arrow_array::{
    Array, ArrayRef, BooleanArray, RecordBatch, StringArray, UInt32Array, UInt64Array,
};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::basic::Compression;
use parquet::file::properties::{WriterProperties, WriterVersion};
use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const DEFAULT_RETENTION_DAYS: u64 = 30;
pub const DEFAULT_SINCE: &str = "24h";
/// Default watchdog budget for one `observe` invocation.
pub const DEFAULT_OBSERVE_TIMEOUT_SEC: u64 = crate::schedule::DEFAULT_OBSERVE_TIMEOUT_SECS;
/// Delta files beyond this count trigger compaction into a single file.
const COMPACTION_THRESHOLD: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrowthConfig {
    pub retention_days: u64,
    pub since: String,
    /// Watchdog budget for one `observe` invocation (item 3 of #31).
    pub observe_timeout_sec: u64,
}

impl Default for GrowthConfig {
    fn default() -> Self {
        Self {
            retention_days: DEFAULT_RETENTION_DAYS,
            since: DEFAULT_SINCE.to_string(),
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
        for i in 0..batch.num_rows() {
            rows.push(StoredRow {
                project_id: project_id.value(i).to_string(),
                worktree_id: worktree_id.value(i).to_string(),
                kind: kind.value(i).to_string(),
                rel_path: rel_path.value(i).to_string(),
                bytes: bytes.value(i),
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
                        present: prev.present,
                        observed_at: prev.observed_at,
                        regrowth_count: prev.regrowth_count,
                    });
                    prev.bytes = obs.bytes;
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
                    growth_bytes: None,
                    regrowth_count: 0,
                    observed_at: 0,
                    confidence: Confidence::High,
                    source: Source::new("test"),
                    note: None,
                }],
                signals: vec![],
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
