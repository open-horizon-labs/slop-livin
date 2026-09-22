//! R4: growth since previous observation, from a reverse-delta store.
//!
//! Reuses the existing zstd/Parquet `Store` (see `store.rs`) rather than
//! adding a second persistence layer. Layout under
//! `${SWAMP_DIR}/<root-scope-id>/`:
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
    Array, ArrayRef, BooleanArray, Int32Array, Int64Array, RecordBatch, StringArray, UInt32Array,
    UInt64Array,
};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::{WriterProperties, WriterVersion};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::os::unix::ffi::OsStrExt;
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

fn should_compact(files: &[PathBuf]) -> bool {
    if files.len() > COMPACTION_THRESHOLD {
        return true;
    }
    // Amortize repeated schema/footer costs for tiny reverse deltas, without
    // repeatedly merging large history runs on every observation.
    files.len() >= 8
        && files
            .iter()
            .try_fold(0u64, |n, p| {
                fs::metadata(p).map(|m| n.saturating_add(m.len()))
            })
            .is_ok_and(|bytes| bytes <= 128 * 1024)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrowthConfig {
    pub retention_days: u64,
    pub since: String,
    pub large_file_min_bytes: u64,
    /// Watchdog budget for one `observe` invocation (item 3 of #31).
    pub observe_timeout_sec: u64,
    /// The `[scan]` table: built-in defaults, includes, excludes, and
    /// disabled detectors (#41). See `crate::scope`.
    pub scan: crate::scope::ScanConfig,
}

impl Default for GrowthConfig {
    fn default() -> Self {
        Self {
            retention_days: DEFAULT_RETENTION_DAYS,
            since: DEFAULT_SINCE.to_string(),
            large_file_min_bytes: DEFAULT_LARGE_FILE_MIN_BYTES,
            observe_timeout_sec: DEFAULT_OBSERVE_TIMEOUT_SEC,
            scan: crate::scope::ScanConfig::default(),
        }
    }
}

impl GrowthConfig {
    /// The file contents that reproduce this configuration, every key
    /// written out with its meaning, so `config init` leaves something a
    /// human can edit.
    pub fn to_toml(&self) -> String {
        format!(
            "# swamp configuration. Every key is optional; these are the effective values.\n\
# How far back growth is measured by default (\"24h\", \"7d\"); --since overrides per call.\n\
since = \"{}\"\n\
# Days of observation history kept in the store before deltas are pruned.\n\
retention_days = {}\n\
# Files at least this large are tracked individually under --dirs.\n\
large_file_min_bytes = {}\n\
# Watchdog budget for one `observe` run, in seconds.\n\
observe_timeout_sec = {}\n\
{}",
            self.since,
            self.retention_days,
            self.large_file_min_bytes,
            self.observe_timeout_sec,
            self.scan.to_toml_table(),
        )
    }
}

/// Raw `config.toml` shape for `toml::from_str`. Every field optional so
/// a config naming only a subset of keys still parses; unknown top-level
/// keys are accepted (forward-compatible), but a key with the wrong
/// *type* (a `[scan]` table where `defaults` is a string, `exclude` is
/// not an array of strings, ...) is a real parse error, not silently
/// discarded -- see `load_config_checked`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
struct RawConfig {
    since: String,
    retention_days: u64,
    large_file_min_bytes: u64,
    observe_timeout_sec: u64,
    scan: crate::scope::ScanConfig,
}

impl Default for RawConfig {
    fn default() -> Self {
        let d = GrowthConfig::default();
        Self {
            since: d.since,
            retention_days: d.retention_days,
            large_file_min_bytes: d.large_file_min_bytes,
            observe_timeout_sec: d.observe_timeout_sec,
            scan: d.scan,
        }
    }
}

impl From<RawConfig> for GrowthConfig {
    fn from(r: RawConfig) -> Self {
        Self {
            since: r.since,
            retention_days: r.retention_days,
            large_file_min_bytes: r.large_file_min_bytes,
            observe_timeout_sec: r.observe_timeout_sec,
            scan: r.scan,
        }
    }
}

/// Reads and validates `<swamp_dir>/config.toml` with a real TOML
/// parser. A missing file is `Ok(GrowthConfig::default())` -- absent is
/// not invalid. A file that exists but fails to parse (bad TOML syntax,
/// or a `[scan]` field with the wrong type, e.g. `defaults = "yes"`
/// instead of a bool) is `Err`: callers that determine scan scope from
/// this must refuse to run rather than silently falling back to
/// (broader) defaults. See `load_config` for the infallible variant used
/// deep in the report pipeline, which only ever reads the four scalar
/// keys and tolerates a malformed file the same way it always has.
pub fn load_config_checked(swamp_dir: &Path) -> Result<GrowthConfig> {
    let path = swamp_dir.join("config.toml");
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(GrowthConfig::default()),
        Err(e) => return Err(e).context(format!("reading {}", path.display())),
    };
    let raw: RawConfig =
        toml::from_str(&text).with_context(|| format!("invalid config at {}", path.display()))?;
    Ok(raw.into())
}

/// Infallible convenience wrapper around [`load_config_checked`] for the
/// report pipeline's internal, scalar-only readers (retention/since/
/// large-file-min-bytes/observe-timeout): a malformed file falls back to
/// defaults for these settings exactly as before real-TOML parsing was
/// added. Scope-resolving call sites (the CLI's `scope`/`report`/
/// `observe`/`ui`/`schedule` commands and `config show`/`init`) must use
/// [`load_config_checked`] instead so invalid `[scan]` config is a
/// visible, nonzero-exit error rather than a silently broadened scope
/// (#41's core requirement) -- see `crates/cli/src/main.rs`.
pub fn load_config(swamp_dir: &Path) -> GrowthConfig {
    load_config_checked(swamp_dir).unwrap_or_default()
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
    if kind.starts_with("Nested:") {
        return kind.to_string();
    }
    format!("{project_id}\u{1}{worktree_id}\u{1}{kind}\u{1}{rel_path}")
}

/// One stored row. Used both for `current.parquet` (where `bytes`/
/// `present` are the latest known value) and for delta files (where they
/// are the *previous* value, before the observation at `observed_at`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredRow {
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

/// Writes a Parquet file so an interrupted write can never leave a
/// corrupt one behind: the rows go to a sibling temp file, which is
/// renamed over `path` only after the writer closed cleanly. A process
/// killed mid-write (this happened: a SIGKILL during `observe` left a
/// `current.parquet` whose footer never landed, and every later run died
/// reading it) loses the new observation, never the store.
/// The one place this crate builds a Parquet writer, so "every
/// observation is zstd-compressed" is structural rather than a habit:
/// callers choose a level, never a codec.
fn write_parquet_atomic(
    path: &Path,
    schema: Arc<Schema>,
    batch: &RecordBatch,
    zstd_level: i32,
) -> Result<()> {
    write_parquet_batches_atomic(path, schema, std::iter::once(Ok(batch.clone())), zstd_level)
}

/// Atomic columnar writer for history batches.
pub(crate) fn write_parquet_batches_atomic(
    path: &Path,
    schema: Arc<Schema>,
    batches: impl IntoIterator<Item = Result<RecordBatch>>,
    zstd_level: i32,
) -> Result<()> {
    let properties = default_zstd_properties(zstd_level);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // Unique even for concurrent writes within the same process/second.
    // RAII removes a failed partial stream; readers retain the prior file.
    let tmp = tempfile::NamedTempFile::new_in(path.parent().unwrap_or(Path::new(".")))?;
    {
        let file = tmp.reopen()?;
        let mut writer = ArrowWriter::try_new(file, schema, Some(properties))?;
        for batch in batches {
            writer.write(&batch?)?;
            writer.flush()?;
        }
        // `close` writes the footer and the trailing magic; until it
        // returns the file is not a Parquet file at all.
        writer.close()?;
    }
    tmp.as_file().sync_all()?;
    tmp.persist(path)
        .map_err(|e| e.error)
        .with_context(|| format!("publish {}", path.display()))?;
    Ok(())
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
    write_parquet_atomic(path, schema, &batch, ARTIFACT_ZSTD_LEVEL)
}

fn read_rows(path: &Path) -> Result<Vec<StoredRow>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .and_then(|b| b.build())
        .with_context(|| {
            format!(
                "read {} (delete it to rebuild this store from a full walk)",
                path.display()
            )
        })?;
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
fn downcast_i64<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a Int64Array> {
    batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<Int64Array>())
        .with_context(|| format!("column {name} is not Int64"))
}
fn downcast_bool<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a BooleanArray> {
    batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<BooleanArray>())
        .with_context(|| format!("column {name} is not Boolean"))
}

fn volume_dir(swamp_dir: &Path, volume_id: u64) -> PathBuf {
    swamp_dir.join(volume_id.to_string())
}

/// Returns the stable store key for one observed root.
///
/// A device is not a sufficient scope: several checkouts (and linked
/// worktrees) commonly share one volume.  The canonical root makes aliases
/// such as `/tmp/work` and `/private/tmp/work` share history while keeping
/// sibling roots independent.  Hashing keeps the existing compact directory
/// layout and avoids putting user paths into the store name.
pub fn root_scoped_volume_id(root: &Path) -> u64 {
    let canonical = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let device = fs::metadata(&canonical)
        .map(|m| m.dev())
        .unwrap_or_default();
    let mut hasher = blake3::Hasher::new();
    hasher.update(&device.to_le_bytes());
    // The canonical path is identity data, not display text. Lossy UTF-8
    // conversion can collapse distinct non-UTF-8 roots into one store.
    hasher.update(canonical.as_os_str().as_bytes());
    let digest = hasher.finalize();
    u64::from_le_bytes(
        digest.as_bytes()[..8]
            .try_into()
            .expect("blake3 digest is 32 bytes"),
    )
}
fn current_path(dir: &Path) -> PathBuf {
    dir.join("current.parquet")
}
fn deltas_dir(dir: &Path) -> PathBuf {
    dir.join("deltas")
}

fn list_delta_files(dir: &Path) -> Vec<PathBuf> {
    list_files_in(&deltas_dir(dir))
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
    mtime_max: u64,
    hardlinked: bool,
    dedup_stale: bool,
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
                let kind = observed_kind(artifact);
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
                    mtime_max: artifact.mtime_max,
                    hardlinked: artifact.hardlinked,
                    dedup_stale: artifact.dedup_stale,
                    rel_path: rel_path_str,
                    bytes: artifact.bytes,
                    local_bytes: if artifact.local_bytes == 0
                        && artifact.source.tool != "cargo.layout"
                    {
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

fn observed_kind(artifact: &ArtifactRow) -> String {
    if artifact.source.tool == "cargo.layout" {
        artifact
            .note
            .as_deref()
            .and_then(|n| n.strip_prefix("nested-id="))
            .map(|id| format!("Nested:{id}"))
            .unwrap_or_else(|| format!("{:?}", artifact.kind))
    } else {
        format!("{:?}", artifact.kind)
    }
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
    swamp_dir: &Path,
    volume_id: u64,
    projects: &mut [ProjectRow],
    observed_at: u64,
    retention_days: u64,
    since_secs: u64,
) -> Result<()> {
    let dir = volume_dir(swamp_dir, volume_id);
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
    // One pass over current + deltas for every key, not one pass per
    // artifact (that was ~300 × 4 Parquet reads per observation).
    let history_index = build_history_index(&dir, retention_days, observed_at)?;
    let empty: Vec<(u64, u64, bool)> = Vec::new();
    for project in projects.iter_mut() {
        for worktree in project.worktrees.iter_mut() {
            for artifact in worktree.artifacts.iter_mut() {
                let rel_path = artifact
                    .path
                    .strip_prefix(&worktree.path)
                    .unwrap_or(&artifact.path)
                    .to_path_buf();
                let kind = observed_kind(artifact);
                let key = row_key(
                    &project.project_id,
                    &worktree.worktree_id,
                    &kind,
                    &rel_path.display().to_string(),
                );
                let history = history_index.get(&key).unwrap_or(&empty);
                artifact.growth_bytes = (!artifact.dedup_stale)
                    .then(|| growth_since(history, artifact.bytes, target_time))
                    .flatten();
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
/// `swamp_dir` is the top-level store root (e.g.
/// `${SWAMP_DIR}`); the caller supplies the root-scope key in `volume_id`.
///
/// `protected_worktree_ids` (#42) names worktree ids this observation
/// could not confirm one way or the other -- typically because access to
/// the worktree's path was lost between observations (see
/// `compute_unconfirmed_worktrees`). A row belonging to one of these
/// worktree ids is never tombstoned by this call even though it is
/// absent from `projects`: absence here means "not observed", not
/// "deleted". See `.oh/guardrails/coverage-changes-are-not-storage-changes.md`.
pub fn observe_and_annotate(
    swamp_dir: &Path,
    volume_id: u64,
    projects: &mut [ProjectRow],
    observed_at: u64,
    retention_days: u64,
    since_secs: u64,
    protected_worktree_ids: &HashSet<String>,
) -> Result<()> {
    let dir = volume_dir(swamp_dir, volume_id);
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
    let mut current_changed = false;
    let mut delta_rows: Vec<StoredRow> = Vec::new();

    for obs in &observed {
        seen_keys.insert(obs.key.clone());
        match current.get_mut(&obs.key) {
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
                    current_changed = true;
                }
                let changed =
                    prev.bytes != obs.bytes || !prev.present || prev.dedup_stale != obs.dedup_stale;
                if changed {
                    current_changed = true;
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
                        mtime_max: prev.mtime_max,
                        hardlinked: prev.hardlinked,
                        dedup_stale: prev.dedup_stale,
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
                prev.dedup_stale = obs.dedup_stale;
                prev.hardlinked = obs.hardlinked;
                prev.mtime_max = obs.mtime_max;
                prev.local_bytes = obs.local_bytes;
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
                current_changed = true;
                current.insert(
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

    // Rows present before, absent now: tombstone them (kept in the
    // store so a later reappearance counts as regrowth), but never
    // emitted as report rows in this issue. A row whose worktree could
    // not be confirmed this pass (#42) is left untouched instead: its
    // absence from `seen_keys` reflects lost access, not deletion.
    for (key, row) in current.iter_mut() {
        if row.present
            && !seen_keys.contains(key)
            && !protected_worktree_ids.contains(&row.worktree_id)
        {
            delta_rows.push(StoredRow {
                project_id: row.project_id.clone(),
                worktree_id: row.worktree_id.clone(),
                kind: row.kind.clone(),
                rel_path: row.rel_path.clone(),
                bytes: row.bytes,
                local_bytes: row.local_bytes,
                mtime_max: row.mtime_max,
                hardlinked: row.hardlinked,
                dedup_stale: row.dedup_stale,
                present: row.present,
                observed_at: row.observed_at,
                regrowth_count: row.regrowth_count,
            });
            row.present = false;
            current_changed = true;
            row.bytes = 0;
            row.observed_at = observed_at;
        }
    }

    // Compute growth/regrowth for the artifacts in *this* report before
    // writing, using the pre-write history (current file on disk plus
    // any not-yet-written delta files already on disk). One index for
    // every key, not one pass over the Parquet files per artifact.
    let target_time = observed_at.saturating_sub(since_secs);
    let history_index = build_history_index(&dir, retention_days, observed_at)?;
    for project in projects.iter_mut() {
        for worktree in project.worktrees.iter_mut() {
            for artifact in worktree.artifacts.iter_mut() {
                let rel_path = artifact
                    .path
                    .strip_prefix(&worktree.path)
                    .unwrap_or(&artifact.path)
                    .to_path_buf();
                let kind = observed_kind(artifact);
                let key = row_key(
                    &project.project_id,
                    &worktree.worktree_id,
                    &kind,
                    &rel_path.display().to_string(),
                );
                let history = history_index.get(&key).cloned().unwrap_or_default();
                artifact.growth_bytes = (!artifact.dedup_stale)
                    .then(|| growth_since(&history, artifact.bytes, target_time))
                    .flatten();
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
    if current_changed {
        write_rows(&current_file, &current_rows)?;
    }

    compact_if_needed(&dir, retention_days, observed_at)?;

    Ok(())
}

/// One historical snapshot of a row's value: `(observed_at, bytes,
/// present)`, oldest first, ending with the value on disk right now
/// (before this observation's write).
/// `(observed_at, bytes, measurement_usable)` history for every key in the store,
/// from the current file plus every delta within retention, sorted by
/// time. Built once per observation.
// (observation time, last measured bytes, measurement usable at that time).
type HistoryIndex = HashMap<String, Vec<(u64, u64, bool)>>;

fn build_history_index(dir: &Path, retention_days: u64, now: u64) -> Result<HistoryIndex> {
    let retention_secs = retention_days.saturating_mul(86400);
    let horizon = now.saturating_sub(retention_secs);
    let mut index: HashMap<String, Vec<(u64, u64, bool)>> = HashMap::new();
    for row in read_rows(&current_path(dir))? {
        index
            .entry(row_key(
                &row.project_id,
                &row.worktree_id,
                &row.kind,
                &row.rel_path,
            ))
            .or_default()
            .push((row.observed_at, row.bytes, !row.present || !row.dedup_stale));
    }
    for delta_path in list_delta_files(dir) {
        for row in read_rows(&delta_path)? {
            if row.observed_at < horizon {
                continue;
            }
            index
                .entry(row_key(
                    &row.project_id,
                    &row.worktree_id,
                    &row.kind,
                    &row.rel_path,
                ))
                .or_default()
                .push((row.observed_at, row.bytes, !row.present || !row.dedup_stale));
        }
    }
    for values in index.values_mut() {
        values.sort_by_key(|(t, _, _)| *t);
    }
    Ok(index)
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
    closest.2.then_some(bytes_now as i64 - closest.1 as i64)
}

/// Merges every delta file into one, dropping deltas older than the
/// retention window, once the delta file count crosses
/// [`COMPACTION_THRESHOLD`].
fn compact_if_needed(dir: &Path, retention_days: u64, now: u64) -> Result<()> {
    let files = list_delta_files(dir);
    if !should_compact(&files) {
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
        write_rows(&next_delta_path(dir), &merged)?;
    }
    // Publish the completed replacement before retiring any source file.
    for path in &files {
        fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
    }
    Ok(())
}

/// Prunes delta files that fall entirely outside the retention window,
/// without waiting for the compaction threshold. Exposed for callers
/// (or a future maintenance command) that want retention enforced on
/// every observation regardless of file count.
/// The oldest observation the store can still answer from, in seconds
/// before `now`: the earliest `observed_at` across the delta log and the
/// current state. `None` when the store holds no observation.
///
/// A growth window longer than this span cannot be honored — the tool
/// would be reporting "growth over a year" from three days of history —
/// so surfaces clamp their offered windows to it and say what they have.
pub fn history_span_secs(dir: &Path, now: u64) -> Option<u64> {
    let mut oldest: Option<u64> = None;
    let mut consider = |rows: Vec<StoredRow>| {
        for r in rows {
            oldest = Some(oldest.map_or(r.observed_at, |o: u64| o.min(r.observed_at)));
        }
    };
    if let Ok(rows) = read_rows(&current_path(dir)) {
        consider(rows);
    }
    for f in list_delta_files(dir) {
        if let Ok(rows) = read_rows(&f) {
            consider(rows);
        }
    }
    oldest.map(|o| now.saturating_sub(o))
}

/// `history_span_secs` for `root`, so every surface bounds its growth windows
/// identically without mixing roots on the same device.
/// One bucketed byte history: `None` before the first observation.
pub type Series = Vec<Option<u64>>;

/// Byte history per artifact row as a step series sampled at `buckets`
/// evenly spaced times over the last `window_secs`, read straight from
/// the reverse-delta log: the current row gives the latest value, each
/// delta row the value that held until the next observation. A bucket
/// before the row's first observation is `None` (not yet observed — not
/// zero); a row recorded absent (`present == false`) is `Some(0)`. Also
/// returns the total over all rows per bucket, `None` where nothing at
/// all had been observed yet. This is what a sparkline draws — no
/// rescan, just the store.
pub fn history_series(
    dir: &Path,
    window_secs: u64,
    buckets: usize,
    now: u64,
) -> (HashMap<String, Series>, Series) {
    let buckets = buckets.max(2);
    let mut points: HashMap<String, Vec<(u64, Option<u64>)>> = HashMap::new();
    let mut push = |rows: Vec<StoredRow>| {
        for r in rows {
            let key = row_key(&r.project_id, &r.worktree_id, &r.kind, &r.rel_path);
            let bytes = if !r.present {
                Some(0)
            } else if r.dedup_stale {
                None
            } else {
                Some(r.bytes)
            };
            points.entry(key).or_default().push((r.observed_at, bytes));
        }
    };
    for f in list_delta_files(dir) {
        if let Ok(rows) = read_rows(&f) {
            push(rows);
        }
    }
    // Current wins ties when several observations share a second.
    if let Ok(rows) = read_rows(&current_path(dir)) {
        push(rows);
    }
    let start = now.saturating_sub(window_secs);
    let step = (window_secs.max(1) as f64) / ((buckets - 1) as f64);
    let times: Vec<u64> = (0..buckets)
        .map(|i| start + (i as f64 * step).round() as u64)
        .collect();
    let mut series: HashMap<String, Series> = HashMap::with_capacity(points.len());
    let mut total: Series = vec![None; buckets];
    let mut stale_total = vec![false; buckets];
    for (key, mut pts) in points {
        pts.sort_by_key(|(t, _)| *t);
        let mut out = Vec::with_capacity(buckets);
        for (i, t) in times.iter().enumerate() {
            let point = pts.iter().rev().find(|(pt, _)| pt <= t);
            let v = point.and_then(|(_, b)| *b);
            if point.is_some() && v.is_none() && !key.starts_with("Nested:") {
                stale_total[i] = true;
            }
            out.push(v);
            if let Some(v) = v
                && !key.starts_with("Nested:")
            {
                total[i] = Some(total[i].unwrap_or(0) + v);
            }
        }
        series.insert(key, out);
    }
    for (i, stale) in stale_total.into_iter().enumerate() {
        if stale {
            total[i] = None;
        }
    }
    (series, total)
}

/// The store key for a report row, so surfaces can look up its series.
pub fn series_key(project_id: &str, worktree_id: &str, kind: &str, rel_path: &str) -> String {
    row_key(project_id, worktree_id, kind, rel_path)
}

/// The root-scoped store directory for `root`.
pub fn volume_store_dir(swamp_dir: &Path, root: &Path) -> PathBuf {
    volume_dir(swamp_dir, root_scoped_volume_id(root))
}

pub fn history_span_for_root(store: &Path, root: &Path, now: u64) -> Option<u64> {
    history_span_secs(&store.join(root_scoped_volume_id(root).to_string()), now)
}

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

/// The artifacts store is small and read on every observation, so it
/// gets the same level the directory base file does.
const ARTIFACT_ZSTD_LEVEL: i32 = 9;
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
        .filter(|p| p.is_file())
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

fn default_zstd_properties(level: i32) -> WriterProperties {
    let level = ZstdLevel::try_new(level).unwrap_or_default();
    WriterProperties::builder()
        .set_compression(Compression::ZSTD(level))
        .set_writer_version(WriterVersion::PARQUET_2_0)
        .build()
}

// --- dirs.parquet ---

#[derive(Debug, Clone, PartialEq, Eq)]
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
    write_parquet_atomic(path, schema, &batch, zstd_level)
}

fn read_dir_rows(path: &Path) -> Result<Vec<StoredDirRow>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .and_then(|b| b.build())
        .with_context(|| {
            format!(
                "read {} (delete it to rebuild this store from a full walk)",
                path.display()
            )
        })?;
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
    swamp_dir: &Path,
    volume_id: u64,
    dirs: &mut [crate::report::DirRollup],
    observed_at: u64,
    retention_days: u64,
    since_secs: u64,
) -> Result<()> {
    let dir = volume_dir(swamp_dir, volume_id);
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
    swamp_dir: &Path,
    volume_id: u64,
    dirs: &mut [crate::report::DirRollup],
    observed_at: u64,
    retention_days: u64,
    since_secs: u64,
) -> Result<()> {
    let dir = volume_dir(swamp_dir, volume_id);
    fs::create_dir_all(&dir)?;
    let current_file = dirs_current_path(&dir);

    let trace = std::env::var("SWAMP_TRACE").is_ok_and(|v| v != "0" && !v.is_empty());
    let t = std::time::Instant::now();
    let mut current: HashMap<String, StoredDirRow> = read_dir_rows(&current_file)?
        .into_iter()
        .map(|r| (dir_row_key(&r.worktree_id, &r.rel_path), r))
        .collect();

    if trace {
        eprintln!(
            "[trace]   dirs: read current ({} rows): {:?}",
            current.len(),
            t.elapsed()
        );
    }
    let t = std::time::Instant::now();
    let history_index = build_dir_history_index(&dir, &current, retention_days, observed_at)?;
    if trace {
        eprintln!("[trace]   dirs: history index: {:?}", t.elapsed());
    }
    let t = std::time::Instant::now();
    let empty_history: Vec<(u64, u64)> = Vec::new();

    // Rewriting the whole current file (zstd-9, tens of thousands of
    // rows) is the dominant cost of an observation where almost nothing
    // moved. Skip it when no row changed, was added, or went absent.
    let mut current_changed = false;
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
                    current_changed = true;
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
                current_changed = true;
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
    if trace {
        eprintln!("[trace]   dirs: diff + annotate: {:?}", t.elapsed());
    }
    let t = std::time::Instant::now();
    if current_changed {
        write_dir_rows(&current_file, &current_rows, DIR_BASE_ZSTD_LEVEL)?;
    }
    if trace {
        eprintln!(
            "[trace]   dirs: write current (zstd-{DIR_BASE_ZSTD_LEVEL}): {:?}",
            t.elapsed()
        );
    }
    let t = std::time::Instant::now();

    compact_dir_deltas_if_needed(&dir, retention_days, observed_at)?;
    if trace {
        eprintln!("[trace]   dirs: compact: {:?}", t.elapsed());
    }
    Ok(())
}

fn compact_dir_deltas_if_needed(dir: &Path, retention_days: u64, now: u64) -> Result<()> {
    let deltas_dir_path = dirs_deltas_dir(dir);
    let files = list_files_in(&deltas_dir_path);
    if !should_compact(&files) {
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
    if !merged.is_empty() {
        merged.sort_by(|a, b| {
            (&a.worktree_id, &a.rel_path, a.observed_at).cmp(&(
                &b.worktree_id,
                &b.rel_path,
                b.observed_at,
            ))
        });
        write_dir_rows(
            &next_seq_path(&deltas_dir_path, "delta-"),
            &merged,
            DIR_DELTA_ZSTD_LEVEL,
        )?;
    }
    for path in &files {
        fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
    }
    Ok(())
}

// --- files.parquet ---

#[derive(Debug, Clone, PartialEq, Eq)]
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
    write_parquet_atomic(path, schema, &batch, zstd_level)
}

fn read_file_rows(path: &Path) -> Result<Vec<StoredFileRow>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .and_then(|b| b.build())
        .with_context(|| {
            format!(
                "read {} (delete it to rebuild this store from a full walk)",
                path.display()
            )
        })?;
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
    swamp_dir: &Path,
    volume_id: u64,
    files: &mut [crate::report::FileRow],
    observed_at: u64,
    retention_days: u64,
    since_secs: u64,
) -> Result<()> {
    let dir = volume_dir(swamp_dir, volume_id);
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
    swamp_dir: &Path,
    volume_id: u64,
    files: &mut [crate::report::FileRow],
    observed_at: u64,
    retention_days: u64,
    since_secs: u64,
) -> Result<()> {
    let dir = volume_dir(swamp_dir, volume_id);
    fs::create_dir_all(&dir)?;
    let current_file = files_current_path(&dir);

    let mut current: HashMap<String, StoredFileRow> = read_file_rows(&current_file)?
        .into_iter()
        .map(|r| (file_row_key(&r.worktree_id, &r.rel_path), r))
        .collect();

    let history_index = build_file_history_index(&dir, &current, retention_days, observed_at)?;
    let empty_history: Vec<(u64, u64)> = Vec::new();

    let mut current_changed = false;
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
                    current_changed = true;
                    delta_rows.push(prev.clone());
                    prev.allocated = row.allocated;
                    prev.mod_time_min = row.mod_time_min;
                    prev.observed_at = observed_at;
                }
            }
            None => {
                current_changed = true;
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
    if current_changed {
        write_file_rows(&current_file, &current_rows, DIR_BASE_ZSTD_LEVEL)?;
    }

    compact_file_deltas_if_needed(&dir, retention_days, observed_at)?;
    Ok(())
}

fn compact_file_deltas_if_needed(dir: &Path, retention_days: u64, now: u64) -> Result<()> {
    let deltas_dir_path = files_deltas_dir(dir);
    let files = list_files_in(&deltas_dir_path);
    if !should_compact(&files) {
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
    if !merged.is_empty() {
        merged.sort_by(|a, b| {
            (&a.worktree_id, &a.rel_path, a.observed_at).cmp(&(
                &b.worktree_id,
                &b.rel_path,
                b.observed_at,
            ))
        });
        write_file_rows(
            &next_seq_path(&deltas_dir_path, "delta-"),
            &merged,
            DIR_DELTA_ZSTD_LEVEL,
        )?;
    }
    for path in &files {
        fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
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
        "Ignored" => ArtifactKind::Ignored,
        "Untracked" => ArtifactKind::Untracked,
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
        .filter(|r| r.present && !is_docker_kind(&r.kind) && !r.kind.starts_with("Nested:"))
    {
        attributed_total += row.bytes;
        artifacts_by_worktree
            .entry(row.worktree_id.clone())
            .or_default()
            .push(ArtifactRow {
                kind: parse_artifact_kind(&row.kind),
                path: PathBuf::from(&row.rel_path),
                bytes: row.bytes,
                mtime_max: row.mtime_max,
                ecosystem: None,
                hardlinked: row.hardlinked,
                dedup_stale: row.dedup_stale,
                local_bytes: row.local_bytes,
                allocated_bytes: None,
                allocated_growth_bytes: None,
                track: None,
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
                evidence: Vec::new(),
            });
    }

    let dirs: Vec<DirRollup> = read_dir_rows(&dirs_current_path(dir))?
        .into_iter()
        .map(|r| DirRollup {
            worktree_id: r.worktree_id,
            track: None,
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
    pub changed_paths: Option<Vec<PathBuf>>,
    /// This root's trusted event window, when the replay earned one:
    /// the replay's **unfiltered** change list and the observation time
    /// it replays from.
    ///
    /// Distinct from `changed_paths`, which is the same replay narrowed
    /// to the subtrees this walk was allowed to descend into. The unit
    /// families (`crate::external`, `crate::agents`) measure paths this
    /// walk deliberately pruned -- every external location nested under
    /// a scan root is pruned from it precisely so it can be measured
    /// once, as its own unit -- so narrowing the list for them would
    /// hand out a window that cannot see a change it was asked about.
    pub event_window: Option<crate::fs_events::TrustedWindow>,
    pub discovered: Vec<DiscoveredWorktree>,
    pub attribution: crate::attribution::AttributionResult,
    /// `"incremental"` or `"full"`.
    pub mode: &'static str,
    /// `"incremental"` on success; otherwise the refusal reason (see
    /// [`crate::fs_events::RefreshRefusal::as_str`]), or `"no_stored_event_id"`
    /// / `"full_forced"` for the two non-FSEvents reasons a walk is full.
    pub reason: &'static str,
    pub changed_dirs: usize,
    /// Worktree ids this observation actually re-walked (incremental
    /// path); `None` on a full walk, meaning all of them. Downstream
    /// stages may carry forward what they computed last time for every
    /// worktree not listed.
    pub rewalked: Option<Vec<String>>,
    /// Incremental accounting: artifact roots re-sized from interior
    /// rows, artifact roots re-sized whole, Source directories re-listed
    /// in place, worktrees handed back to the walker.
    pub in_place: (usize, usize, usize, usize),
    /// Worktree ids that were present in the last observation's topology
    /// but are absent from `discovered` this pass *and could not be
    /// confirmed gone* -- the path still exists on disk but could not be
    /// read (e.g. `chmod 000`), so its absence from `discovered` reflects
    /// lost access, not deletion (#42). The growth store must not
    /// tombstone rows belonging to these worktree ids from this
    /// observation: an inaccessible worktree is a coverage gap, never a
    /// storage change. A worktree id whose path is genuinely gone
    /// (`ENOENT`) is *not* included here -- that is real deletion, and
    /// tombstoning is exactly correct for it.
    pub unconfirmed_worktree_ids: Vec<String>,
}

/// Threshold past which re-walking piecemeal costs more than a full
/// walk: more than this fraction of previously known directories
/// implicated by one replay.
const TOO_MANY_CHANGES_FRACTION: f64 = 0.20;

/// The [`RefreshRefusal::TooSoon`] floor, in seconds. Overridable via
/// `SWAMP_FSEVENTS_MIN_INTERVAL_SECS` so a test driving a canned
/// [`crate::fs_events::FsEventsSource`] -- which has no real FSEvents
/// log-persistence lag to protect against -- can set it to `0` and reach
/// the incremental path without a real `sleep`.
fn min_interval_secs() -> u64 {
    std::env::var("SWAMP_FSEVENTS_MIN_INTERVAL_SECS")
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
    swamp_dir: &Path,
    root: &Path,
    observed_at: u64,
    large_file_min_bytes: u64,
    force_full: bool,
    observe: bool,
) -> Result<TrackedWalk> {
    observe_tracked_with_source(
        swamp_dir,
        root,
        observed_at,
        large_file_min_bytes,
        force_full,
        observe,
        crate::fs_events::platform_source().as_ref(),
        &[],
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
///
/// This low-level entry point commits after the walk. Report pipelines must use
/// [`stage_tracked_with_source`] and commit only after their downstream writes.
#[allow(clippy::too_many_arguments)]
pub fn observe_tracked_with_source(
    swamp_dir: &Path,
    root: &Path,
    observed_at: u64,
    large_file_min_bytes: u64,
    force_full: bool,
    observe: bool,
    source: &dyn crate::fs_events::FsEventsSource,
    excluded: &[PathBuf],
) -> Result<TrackedWalk> {
    let (walk, checkpoint) = stage_tracked_with_source(
        swamp_dir,
        root,
        observed_at,
        large_file_min_bytes,
        force_full,
        observe,
        source,
        excluded,
    )?;
    if let Some(checkpoint) = checkpoint {
        checkpoint.commit()?;
    }
    Ok(walk)
}

/// Replay state is staged until all observation consumers have persisted their
/// facts. Dropping this value on any later failure leaves the old replay anchor.
pub struct ObservationCheckpoint {
    dir: PathBuf,
    state: Option<FsEventsState>,
    topology: Vec<StoredWorktree>,
    unowned: Vec<crate::report::UnownedRow>,
    /// Linux collector: the dirty-list entries this observation's walk
    /// covered, consumed only once everything above is written.
    consume: Option<crate::continuity::Consumption>,
    /// Held from before the previous state was read until the commit (or
    /// drop): two observations of one root -- TUI, CLI, a scheduled run
    /// -- cannot interleave, so neither overwrites the other's rows with
    /// an older walk or consumes changes the other has not written.
    _lock: Option<crate::continuity::FileLock>,
}

impl ObservationCheckpoint {
    pub fn commit(self) -> Result<()> {
        write_topology(&self.dir, &self.topology)?;
        write_unowned(&self.dir, &self.unowned)?;
        // Publish the replay anchor last. This is safe replay ordering, not an
        // atomic transaction across the legacy volume-wide datasets.
        if let Some(state) = self.state {
            write_fsevents_state(&self.dir, &state)?;
        }
        // And only after that, forget the collector's entries this walk
        // covered. A crash before this line leaves them, and the next
        // observation re-walks them: redundant, never wrong.
        if let Some(c) = &self.consume {
            crate::continuity::consume(c)?;
        }
        Ok(())
    }

    /// Test hook: the crash between the history write and the
    /// consumption -- everything is written except the consumption.
    #[doc(hidden)]
    pub fn commit_without_consuming_for_test(mut self) -> Result<()> {
        self.consume = None;
        self.commit()
    }
}

/// How long an observation waits for another observation of the same
/// root to finish before giving up.
const OBSERVATION_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(600);

#[allow(clippy::too_many_arguments)]
pub fn stage_tracked_with_source(
    swamp_dir: &Path,
    root: &Path,
    observed_at: u64,
    large_file_min_bytes: u64,
    force_full: bool,
    observe: bool,
    source: &dyn crate::fs_events::FsEventsSource,
    excluded: &[PathBuf],
) -> Result<(TrackedWalk, Option<ObservationCheckpoint>)> {
    // This public lower-level entry point must be safe for direct callers;
    // never persist alias-form topology into a canonical root scope.
    let root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let volume_id = root_scoped_volume_id(&root);
    let dir = volume_dir(swamp_dir, volume_id);
    fs::create_dir_all(&dir)?;
    // One writer per root, from reading the previous state to committing
    // the next (see `ObservationCheckpoint::_lock`). A read-only pass
    // (`observe == false`) writes nothing and takes nothing.
    let lock = if observe {
        Some(crate::continuity::lock_wait(
            &dir.join("observation.lock"),
            true,
            OBSERVATION_LOCK_WAIT,
        )?)
    } else {
        None
    };

    // FSEvents and persisted topology use the canonical root namespace.
    let prev_state = read_fsevents_state(&dir);

    // `force_full` (`--full`, and every pre-#29 caller: `report_full`,
    // `report_with*`, every test that predates this feature) must never
    // touch the FSEvents source at all -- not the
    // real one (this crate runs alongside dozens of other concurrent
    // test/CLI processes on a shared machine, where `fseventsd` itself
    // can become the bottleneck under combined load; a `source.replay`
    // call that is merely slow under contention still burns wall time
    // this path has promised never to pay), and not even a canned one in
    // tests (there is nothing to answer). Skipping the call entirely,
    // rather than calling it and discarding the answer, is what actually
    // keeps this path load-free instead of just "load but ignore".
    // Classification rules changed since the store was walked: rows that
    // no longer count (or newly count) as artifacts only get fixed by a
    // walk that visits them, so take the one full walk now.
    let rules_changed = prev_state.rules_version != crate::ecosystem::RULES_VERSION;
    // Read once, ahead of either branch below: both a forced/rules-change
    // full walk and the ordinary incremental-or-full path need it to tell
    // "worktree confirmed gone" from "worktree access lost" (#42).
    let prev_topology_for_check = read_topology(&dir);
    if force_full || rules_changed {
        let reason = if force_full {
            "full_forced"
        } else {
            "full_rules_changed"
        };
        let mut result = full_walk(&root, observed_at, large_file_min_bytes, reason, excluded)?;
        result.unconfirmed_worktree_ids =
            compute_unconfirmed_worktrees(prev_topology_for_check.as_deref(), &result.discovered);
        let checkpoint = observe.then(|| ObservationCheckpoint {
            dir,
            consume: None,
            _lock: lock,
            topology: to_stored_worktrees(&result.discovered),
            unowned: result.attribution.unowned.clone(),
            // The stored FSEvents id/device is deliberately left as-is: a
            // forced full walk has nothing new to report there (no
            // replay ran), and an older stored id just means the next
            // real incremental attempt replays a larger, still-correct
            // window rather than a wrong one. The rules version is
            // stamped so the next call goes incremental again.
            state: rules_changed.then(|| FsEventsState {
                rules_version: crate::ecosystem::RULES_VERSION,
                ..prev_state.clone()
            }),
        });
        return Ok((result, checkpoint));
    }

    // FSEvents' own persisted log can lag a write by longer than the
    // growth store's whole-second timestamp granularity, so a replay
    // requested this soon after the baseline cannot yet distinguish
    // "nothing changed" from "the change has not been logged yet" --
    // most visibly when two observations happen back-to-back (tests;
    // a scripted double-run), where a live FSEvents source can
    // legitimately report zero changes for a write that already
    // happened. Below this floor, skip straight to a full walk rather
    // than trust an answer FSEvents itself cannot yet vouch for.
    // Overridable via `SWAMP_FSEVENTS_MIN_INTERVAL_SECS` so tests
    // that use a canned source (which has no real log-lag to protect
    // against) can set it to `0` and skip real sleeps entirely.
    let too_soon = prev_state
        .last_observed_at
        .is_some_and(|t| observed_at.saturating_sub(t) < min_interval_secs());
    // The instant the window opens from. Without one there is no window
    // at all: a stored row cannot be shown to predate a replay whose
    // start is unknown.
    let window_since = prev_state.last_observed_at;
    let t_replay = std::time::Instant::now();
    let plan = source.replay(&FsEventsRequest {
        root: root.clone(),
        since: prev_state,
        swamp_dir: Some(swamp_dir.to_path_buf()),
        excluded: excluded.to_vec(),
    });
    if std::env::var("SWAMP_TRACE").is_ok_and(|v| v != "0" && !v.is_empty()) {
        eprintln!(
            "[trace] fsevents replay: {:?} (incremental={}, changed_dirs={})",
            t_replay.elapsed(),
            plan.incremental,
            plan.changed_dirs.len()
        );
    }
    let prev_topology = prev_topology_for_check.clone();
    // Prune any FSEvents-reported change that falls inside an excluded
    // subtree (#42) before it ever reaches the incremental re-walk
    // machinery: `apply_incremental`/`attribute_one_worktree`/
    // `discover_shallow` have no exclusion list of their own precisely
    // because nothing excluded is ever supposed to reach them.
    let relevant_changed_dirs: Vec<PathBuf> = if excluded.is_empty() {
        plan.changed_dirs.clone()
    } else {
        plan.changed_dirs
            .iter()
            .filter(|p| !excluded.iter().any(|e| *p == e || p.starts_with(e)))
            .cloned()
            .collect()
    };

    let mut result = if too_soon && !plan.live {
        full_walk(
            &root,
            observed_at,
            large_file_min_bytes,
            crate::fs_events::RefreshRefusal::TooSoon.as_str(),
            excluded,
        )?
    } else if !plan.incremental {
        full_walk(
            &root,
            observed_at,
            large_file_min_bytes,
            plan.reason_str(),
            excluded,
        )?
    } else {
        match prev_topology {
            None => full_walk(
                &root,
                observed_at,
                large_file_min_bytes,
                "no_stored_event_id",
                excluded,
            )?,
            Some(ref topo) => {
                // Floored at a minimum so a tiny tree (a handful of
                // Source directories) doesn't trip the "too many
                // changes" guard on the very first touched file --
                // the guard exists to protect large trees, where a
                // fraction is the meaningful signal.
                let known_dirs = read_dir_rows(&dirs_current_path(&dir))?.len().max(20);
                if relevant_changed_dirs.len() as f64
                    > TOO_MANY_CHANGES_FRACTION * known_dirs as f64
                {
                    full_walk(
                        &root,
                        observed_at,
                        large_file_min_bytes,
                        "too_many_changes",
                        excluded,
                    )?
                } else {
                    apply_incremental(
                        topo,
                        &relevant_changed_dirs,
                        observed_at,
                        large_file_min_bytes,
                        &dir,
                    )?
                }
            }
        }
    };
    result.unconfirmed_worktree_ids =
        compute_unconfirmed_worktrees(prev_topology_for_check.as_deref(), &result.discovered);
    // A window only where the incremental path was actually taken: a
    // full walk, a refusal, `too_soon` and `too_many_changes` all mean
    // this pass cannot say what did *not* change, which is exactly the
    // claim a reuse rests on.
    result.event_window = match (result.mode, window_since) {
        ("incremental", Some(since)) => Some((plan.changed_dirs.clone(), since)),
        _ => None,
    };

    let consume = plan.consume.clone();
    let checkpoint = observe.then(|| ObservationCheckpoint {
        consume,
        _lock: lock,
        // Re-anchor for the next call regardless of which path was taken.
        state: Some(FsEventsState {
            event_id: Some(plan.current_event_id),
            device: plan.device,
            last_observed_at: Some(observed_at),
            rules_version: crate::ecosystem::RULES_VERSION,
        }),
        topology: to_stored_worktrees(&result.discovered),
        unowned: result.attribution.unowned.clone(),
        dir,
    });

    Ok((result, checkpoint))
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

/// Merges a worktree's `Ignored` / `Untracked` rows back into its single
/// `Source` row.
///
/// The split is a presentation of one fact — "everything here that is
/// not a classified artifact" — and the incremental walk's arithmetic is
/// written against that one number: it adjusts the remainder by a delta,
/// re-lists directories into it, and carries it forward. Collapsing on
/// the way in and splitting on the way out keeps that arithmetic in one
/// shape, so the store's rows can be split without every incremental
/// code path learning about three of them.
fn collapse_remainder(attribution: &mut crate::attribution::AttributionResult) {
    for rows in attribution.artifacts_by_worktree.values_mut() {
        if !rows
            .iter()
            .any(|r| matches!(r.kind, ArtifactKind::Ignored | ArtifactKind::Untracked))
        {
            continue;
        }
        let mut merged: Option<ArtifactRow> = None;
        rows.retain(|r| {
            if !r.kind.is_worktree_remainder() {
                return true;
            }
            match merged.as_mut() {
                None => {
                    let mut base = r.clone();
                    base.kind = ArtifactKind::Source;
                    base.track = None;
                    merged = Some(base);
                }
                Some(m) => {
                    m.bytes += r.bytes;
                    m.local_bytes += r.local_bytes;
                    m.observed_at = m.observed_at.max(r.observed_at);
                    m.hardlinked |= r.hardlinked;
                    if let (Some(a), Some(b)) = (m.growth_bytes, r.growth_bytes) {
                        m.growth_bytes = Some(a + b);
                    }
                    m.mtime_max = m.mtime_max.max(r.mtime_max);
                }
            }
            false
        });
        if let Some(m) = merged {
            rows.push(m);
        }
    }
}

/// Splits each worktree's remainder row into what git tracks, what a
/// gitignore rule matches, and what is in no version control at all.
///
/// A single row labelled `source` was a lie about every checkout that
/// holds ignored output or private scratch data: those bytes are not
/// authored work and no remote has a copy. Each kind now states its own
/// recovery contract, and the three totals still sum to the one the
/// walk measured — the split apportions that number rather than
/// re-measuring, so no accounting is invented here.
///
/// Leaves the row undivided when the checkout is not a repository, when
/// the walk kept no directory rows for it, or when the split would be
/// entirely one bucket anyway.
fn split_remainder(
    discovered: &[DiscoveredWorktree],
    attribution: &mut crate::attribution::AttributionResult,
) {
    let roots: std::collections::HashMap<String, PathBuf> = discovered
        .iter()
        .map(|d| {
            (
                crate::entities::id_for(&d.path.display().to_string()),
                d.path.clone(),
            )
        })
        .collect();
    for (wt_id, rows) in attribution.artifacts_by_worktree.iter_mut() {
        let Some(root) = roots.get(wt_id) else {
            continue;
        };
        let Some(idx) = rows.iter().position(|r| r.kind == ArtifactKind::Source) else {
            continue;
        };
        let artifact_rels: std::collections::HashSet<String> = rows
            .iter()
            .filter(|r| !r.kind.is_worktree_remainder())
            .map(|r| rel_path_string(root, &r.path))
            .collect();
        let wt_dirs: Vec<crate::report::DirRollup> = attribution
            .dirs
            .iter()
            .filter(|d| &d.worktree_id == wt_id)
            .cloned()
            .collect();
        if wt_dirs.is_empty() {
            continue;
        }
        let wt_files: Vec<crate::report::FileRow> = attribution
            .files
            .iter()
            .filter(|f| &f.worktree_id == wt_id)
            .cloned()
            .collect();
        let Some(split) =
            crate::ignore::split_dirs_by_track(root, &wt_dirs, &wt_files, &artifact_rels)
        else {
            continue;
        };
        let measured = split.total();
        if measured == 0 || split.tracked == measured {
            continue;
        }
        let base = rows.remove(idx);
        // Apportion what the walk measured, rather than substituting the
        // directory rows' own sum: the walk's number is the one every
        // total in the report was built from, and hardlink dedup can
        // make the two differ.
        let parts = [
            (
                ArtifactKind::Source,
                crate::ignore::TrackState::Tracked,
                split.tracked,
            ),
            (
                ArtifactKind::Ignored,
                crate::ignore::TrackState::Ignored,
                split.ignored,
            ),
            (
                ArtifactKind::Untracked,
                crate::ignore::TrackState::Untracked,
                split.untracked,
            ),
        ];
        let largest = parts
            .iter()
            .enumerate()
            .max_by_key(|(_, (_, _, b))| *b)
            .map(|(i, _)| i)
            .unwrap_or(0);
        let share = |total: u64, bucket: u64| -> u64 {
            (total as u128 * bucket as u128 / measured as u128) as u64
        };
        // Every bucket but the largest takes its share; the largest then
        // takes whatever is left, so the parts add back up to exactly
        // what the walk measured no matter how the division rounds.
        let mut bytes: [u64; 3] = [0; 3];
        let mut local: [u64; 3] = [0; 3];
        for (i, (_, _, bucket)) in parts.iter().enumerate() {
            if i == largest {
                continue;
            }
            bytes[i] = share(base.bytes, *bucket);
            local[i] = share(base.local_bytes, *bucket);
        }
        bytes[largest] = base.bytes.saturating_sub(bytes.iter().sum::<u64>());
        local[largest] = base.local_bytes.saturating_sub(local.iter().sum::<u64>());
        for (i, (kind, track, bucket)) in parts.iter().enumerate() {
            if bytes[i] == 0 && *bucket == 0 {
                continue;
            }
            let mut row = base.clone();
            row.kind = kind.clone();
            row.track = Some(*track);
            row.bytes = bytes[i];
            row.local_bytes = local[i];
            // The growth history belongs to the undivided remainder; a
            // share of it would be a number nothing observed.
            row.growth_bytes = None;
            rows.push(row);
        }
    }
}

fn full_walk(
    root: &Path,
    observed_at: u64,
    large_file_min_bytes: u64,
    reason: &'static str,
    excluded: &[PathBuf],
) -> Result<TrackedWalk> {
    let (discovered, mut attribution) =
        crate::walk::discover_and_attribute(root, observed_at, large_file_min_bytes, excluded)?;
    split_remainder(&discovered, &mut attribution);
    Ok(TrackedWalk {
        discovered,
        attribution,
        mode: "full",
        changed_paths: None,
        event_window: None,
        reason,
        changed_dirs: 0,
        rewalked: None,
        in_place: (0, 0, 0, 0),
        unconfirmed_worktree_ids: Vec::new(),
    })
}

/// Worktree ids from `prev` whose path is absent from `discovered` this
/// pass, split into "confirmed gone" (tombstoning is correct) versus
/// "could not confirm" (the path still exists but could not be read, so
/// the growth store must preserve its rows as-is). Only the latter are
/// returned. A single non-recursive `symlink_metadata`/`read_dir` pair
/// per candidate; bounded by the number of worktrees that dropped out of
/// this observation, never by tree size.
fn compute_unconfirmed_worktrees(
    prev: Option<&[StoredWorktree]>,
    discovered: &[DiscoveredWorktree],
) -> Vec<String> {
    let Some(prev) = prev else {
        return Vec::new();
    };
    let discovered_paths: HashSet<&Path> = discovered.iter().map(|d| d.path.as_path()).collect();
    prev.iter()
        .filter(|pw| !discovered_paths.contains(pw.path.as_path()))
        .filter(|pw| match fs::symlink_metadata(&pw.path) {
            // Gone entirely: real deletion, tombstoning is correct.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            // Existed, could not be statted for some other reason (also
            // commonly permission-denied on a parent directory): treat as
            // unconfirmed, the conservative choice.
            Err(_) => true,
            // The path itself still exists. If it can be listed, a real
            // walk would have discovered it, so its absence from
            // `discovered` means it is no longer a git worktree (e.g.
            // `.git` was removed) -- a real change, not a coverage gap.
            // If it cannot be listed, access was lost, not the worktree.
            Ok(_) => fs::read_dir(&pw.path).is_err(),
        })
        .map(|pw| pw.worktree_id.clone())
        .collect()
}

fn rel_path_string(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let s = rel.display().to_string();
    if s == "." { String::new() } else { s }
}

/// `rel` is `root` or lies under it (`root` empty = the worktree root).
fn under(rel: &str, root: &str) -> bool {
    root.is_empty() || rel == root || rel.starts_with(&format!("{root}/"))
}

/// Re-sizes a folded artifact from its stored interior rows and the
/// directories FSEvents named, without walking the rest of it. Each
/// changed directory is re-listed (own bytes, counts, mtime); a vanished
/// directory drops its subtree's rows; a new subdirectory is walked and
/// gets rows. Then the unit's rows are re-aggregated and the root row's
/// total is its path allocation, not a deduplicated count. Returns `None` when the
/// store has no row for the root (older store: caller re-sizes whole).
fn resize_interior(
    wt_root: &Path,
    worktree_id: &str,
    rel_root: &str,
    changed: &[PathBuf],
    dirs: &mut Vec<DirRollup>,
) -> Option<(u64, u64, bool)> {
    use std::os::unix::fs::MetadataExt;
    let mut mtime_max: u64 = 0;
    let mut saw_hardlink = false;
    let mut changed_rels: Vec<String> = changed
        .iter()
        .map(|c| rel_path_string(wt_root, c))
        .filter(|r| under(r, rel_root))
        .collect();
    changed_rels.sort();
    changed_rels.dedup();
    for rel_c in &changed_rels {
        let abs = wt_root.join(rel_c);
        let Ok(meta) = fs::symlink_metadata(&abs) else {
            // Gone: its whole subtree with it.
            dirs.retain(|d| !(d.worktree_id == worktree_id && under(&d.rel_path, rel_c)));
            continue;
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            continue;
        }
        let measured = crate::walk::measure_directory(&abs).ok()?;
        let own = measured.allocated;
        let files = measured.files;
        let subdirs = measured.children.len() as u32;
        let symlinks = measured.symlinks;
        let dir_mtime = meta.mtime().max(measured.mtime);
        saw_hardlink |= measured.hardlinked;
        let on_disk_subdirs: HashSet<String> = measured
            .children
            .into_iter()
            .map(|name| format!("{rel_c}/{name}"))
            .collect();
        mtime_max = mtime_max.max(dir_mtime.max(0) as u64);
        // Children the store knows that are no longer on disk.
        let stored_children: HashSet<String> = dirs
            .iter()
            .filter(|d| d.worktree_id == worktree_id && d.parent_rel_path.as_deref() == Some(rel_c))
            .map(|d| d.rel_path.clone())
            .collect();
        for gone in stored_children
            .iter()
            .filter(|c| !on_disk_subdirs.contains(c.as_str()))
        {
            dirs.retain(|d| !(d.worktree_id == worktree_id && under(&d.rel_path, gone)));
        }
        // Subdirectories on disk the store has never seen: walk them.
        for new_rel in on_disk_subdirs
            .iter()
            .filter(|c| !stored_children.contains(c.as_str()))
        {
            let (measured, rows) = crate::walk::resize_artifact_with_dirs(
                &wt_root.join(new_rel),
                ArtifactKind::Cache,
                0,
                Some((worktree_id, wt_root)),
            );
            saw_hardlink |= measured.hardlinked;
            for r in &rows {
                mtime_max = mtime_max.max((r.mod_time_min as i64 * 60).max(0) as u64);
            }
            dirs.retain(|d| !(d.worktree_id == worktree_id && under(&d.rel_path, new_rel)));
            dirs.extend(rows);
        }
        // This directory's own row.
        let parent_rel_path = if rel_c.is_empty() {
            None
        } else {
            Some(
                rel_c
                    .rsplit_once('/')
                    .map(|(p, _)| p.to_string())
                    .unwrap_or_default(),
            )
        };
        let row = DirRollup {
            worktree_id: worktree_id.to_string(),
            track: None,
            rel_path: rel_c.clone(),
            parent_rel_path,
            allocated_total: own,
            own_allocated: own,
            file_count: files,
            entry_count: files + subdirs + symlinks,
            symlink_count: symlinks,
            mod_time_min: (dir_mtime / 60) as i32,
            complete: true,
            growth_bytes: None,
        };
        if let Some(existing) = dirs
            .iter_mut()
            .find(|d| d.worktree_id == worktree_id && &d.rel_path == rel_c)
        {
            *existing = row;
        } else {
            dirs.push(row);
        }
    }
    // Re-aggregate this unit's rows; the root row's total is the unit.
    let mut interior: Vec<DirRollup> = dirs
        .iter()
        .filter(|d| d.worktree_id == worktree_id && under(&d.rel_path, rel_root))
        .cloned()
        .collect();
    if interior.is_empty() {
        return None;
    }
    crate::report::aggregate_dir_totals(&mut interior, &std::collections::HashSet::new());
    let Some(root_total) = interior
        .iter()
        .find(|d| d.rel_path == rel_root)
        .map(|d| d.allocated_total)
    else {
        if std::env::var("SWAMP_TRACE").is_ok_and(|v| v != "0" && !v.is_empty()) {
            eprintln!(
                "[trace]   interior rows exist ({}) but no root row {rel_root:?}",
                interior.len()
            );
        }
        return None;
    };
    dirs.retain(|d| !(d.worktree_id == worktree_id && under(&d.rel_path, rel_root)));
    dirs.extend(interior);
    Some((root_total, mtime_max, saw_hardlink))
}

/// Re-lists changed Source directories of one worktree from their stored
/// rows and returns the worktree's new Source byte total (the root row's
/// aggregate, artifact roots excluded) plus the artifact roots that
/// vanished with a deleted directory. `None` means the store cannot
/// answer without a walk: a changed directory it has no row for, or a
/// subdirectory it has never seen (which could be a new artifact or a
/// nested checkout).
fn relist_source_dirs(
    wt_root: &Path,
    worktree_id: &str,
    changed: &[PathBuf],
    artifact_rels: &HashSet<String>,
    dirs: &mut Vec<DirRollup>,
    files: &mut Vec<crate::report::FileRow>,
    large_file_min_bytes: u64,
) -> Option<(u64, HashSet<String>)> {
    use std::os::unix::fs::MetadataExt;
    let mut removed_artifacts: HashSet<String> = HashSet::new();
    let mut rels: Vec<String> = changed
        .iter()
        .map(|c| rel_path_string(wt_root, c))
        .collect();
    rels.sort();
    rels.dedup();
    for rel_c in &rels {
        let abs = if rel_c.is_empty() {
            wt_root.to_path_buf()
        } else {
            wt_root.join(rel_c)
        };
        let known = dirs
            .iter()
            .any(|d| d.worktree_id == worktree_id && &d.rel_path == rel_c);
        let Ok(meta) = fs::symlink_metadata(&abs) else {
            if rel_c.is_empty() {
                return None; // the worktree itself is gone; handled by the caller.
            }
            // Deleted: its subtree's rows go, and any artifact rooted in it.
            dirs.retain(|d| !(d.worktree_id == worktree_id && under(&d.rel_path, rel_c)));
            files.retain(|f| !(f.worktree_id == worktree_id && under(&f.rel_path, rel_c)));
            for a in artifact_rels {
                if under(a, rel_c) {
                    removed_artifacts.insert(a.clone());
                }
            }
            continue;
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            continue;
        }
        if !known {
            return None;
        }
        let Ok(entries) = fs::read_dir(&abs) else {
            continue;
        };
        let mut own: u64 = 0;
        let (mut nfiles, mut ndirs, mut nsymlinks) = (0u32, 0u32, 0u32);
        let mut dir_mtime: i64 = meta.mtime();
        let mut on_disk_subdirs: Vec<String> = Vec::new();
        let mut new_files: Vec<crate::report::FileRow> = Vec::new();
        for e in entries.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            if ft.is_symlink() {
                nsymlinks += 1;
                continue;
            }
            let child_rel = if rel_c.is_empty() {
                name.clone()
            } else {
                format!("{rel_c}/{name}")
            };
            if ft.is_dir() {
                // `.git` is the Git artifact root: a stored child like any other.
                ndirs += 1;
                on_disk_subdirs.push(child_rel);
            } else if ft.is_file() {
                let Ok(fm) = fs::symlink_metadata(e.path()) else {
                    continue;
                };
                if fm.file_type().is_symlink() || !fm.is_file() {
                    continue;
                }
                nfiles += 1;
                let bytes = crate::attribution::allocated_bytes(&fm);
                own += bytes;
                dir_mtime = dir_mtime.max(fm.mtime());
                if bytes >= large_file_min_bytes {
                    new_files.push(crate::report::FileRow {
                        worktree_id: worktree_id.to_string(),
                        rel_path: child_rel,
                        allocated: bytes,
                        mod_time_min: (fm.mtime() / 60) as i32,
                        growth_bytes: None,
                    });
                }
            }
        }
        // Children the store knows here: Source dir rows and artifact roots.
        let stored_children: HashSet<String> = dirs
            .iter()
            .filter(|d| d.worktree_id == worktree_id && d.parent_rel_path.as_deref() == Some(rel_c))
            .map(|d| d.rel_path.clone())
            .chain(
                artifact_rels
                    .iter()
                    .filter(|a| {
                        a.rsplit_once('/').map(|(p, _)| p) == Some(rel_c.as_str())
                            || (rel_c.is_empty() && !a.contains('/'))
                    })
                    .cloned(),
            )
            .collect();
        if on_disk_subdirs.iter().any(|c| !stored_children.contains(c)) {
            return None; // something new under here: the walker decides what it is.
        }
        for gone in stored_children
            .iter()
            .filter(|c| !on_disk_subdirs.contains(c))
        {
            dirs.retain(|d| !(d.worktree_id == worktree_id && under(&d.rel_path, gone)));
            files.retain(|f| !(f.worktree_id == worktree_id && under(&f.rel_path, gone)));
            if artifact_rels.contains(gone) {
                removed_artifacts.insert(gone.clone());
            }
        }
        // This directory's own row and its large-file rows.
        let parent_rel_path = if rel_c.is_empty() {
            None
        } else {
            Some(
                rel_c
                    .rsplit_once('/')
                    .map(|(p, _)| p.to_string())
                    .unwrap_or_default(),
            )
        };
        if let Some(existing) = dirs
            .iter_mut()
            .find(|d| d.worktree_id == worktree_id && &d.rel_path == rel_c)
        {
            existing.own_allocated = own;
            existing.allocated_total = own;
            existing.file_count = nfiles;
            existing.entry_count = nfiles + ndirs + nsymlinks;
            existing.symlink_count = nsymlinks;
            existing.mod_time_min = (dir_mtime / 60) as i32;
            existing.parent_rel_path = parent_rel_path;
        }
        files.retain(|f| {
            !(f.worktree_id == worktree_id
                && f.rel_path.rsplit_once('/').map(|(p, _)| p).unwrap_or("") == rel_c.as_str())
        });
        files.extend(new_files);
    }
    // Re-aggregate this worktree's directory rows; the root row's total,
    // with artifact roots excluded from the roll-up, is the Source bytes.
    let mut mine: Vec<DirRollup> = dirs
        .iter()
        .filter(|d| d.worktree_id == worktree_id)
        .cloned()
        .collect();
    let roots: HashSet<(String, String)> = artifact_rels
        .iter()
        .map(|r| (worktree_id.to_string(), r.clone()))
        .collect();
    crate::report::aggregate_dir_totals(&mut mine, &roots);
    let source_total = mine
        .iter()
        .find(|d| d.rel_path.is_empty())
        .map(|d| d.allocated_total)?;
    dirs.retain(|d| d.worktree_id != worktree_id);
    dirs.extend(mine);
    Some((source_total, removed_artifacts))
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
    let trace = std::env::var("SWAMP_TRACE").is_ok_and(|v| v != "0" && !v.is_empty());
    let t0 = std::time::Instant::now();
    let mut attribution = reconstruct_attribution(dir)?;
    // The incremental arithmetic below works on one remainder row per
    // worktree; the store holds it already split. See `collapse_remainder`.
    collapse_remainder(&mut attribution);
    if trace {
        eprintln!("[trace] incremental: reconstruct store: {:?}", t0.elapsed());
    }
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
    let mut source_dirs_to_relist: HashMap<String, Vec<PathBuf>> = HashMap::new();
    let mut artifact_roots_to_resize: HashMap<PathBuf, (String, ArtifactKind, Vec<PathBuf>)> =
        HashMap::new();
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
                        !r.kind.is_worktree_remainder()
                            && (changed == &r.path || changed.starts_with(&r.path))
                    })
                    .max_by_key(|r| r.path.as_os_str().len())
            });
        if let Some(row) = artifact_hit {
            artifact_roots_to_resize
                .entry(row.path.clone())
                .or_insert_with(|| (wt.worktree_id.clone(), row.kind.clone(), Vec::new()))
                .2
                .push(changed.clone());
        } else {
            source_dirs_to_relist
                .entry(wt.worktree_id.clone())
                .or_default()
                .push(changed.clone());
            // A changed directory that gained (or lost) a `.git` inside
            // an already-known worktree's tree is a nested checkout; the
            // worktree-level rewalk below re-sizes but does not itself
            // run project discovery, so scan explicitly too.
            if changed != &wt.path && changed.join(".git").exists() {
                discovery_scan_roots.push(changed.clone());
                worktrees_to_rewalk.insert(wt.worktree_id.clone());
            }
        }
    }
    let mut total_delta: i64 = 0;
    let (mut n_interior, mut n_whole) = (0usize, 0usize);
    // A changed Source directory is re-listed in place from its stored
    // row: own bytes, vanished children dropped, the worktree's Source
    // total re-aggregated. Only a directory the store has never seen (a
    // new subtree, which may be a new artifact or a nested checkout)
    // sends the whole worktree back to the walker.
    let t_relist = std::time::Instant::now();
    let mut relisted = 0usize;
    for (wt_id, changed) in &source_dirs_to_relist {
        if worktrees_to_rewalk.contains(wt_id) {
            continue;
        }
        let Some(root) = worktree_root.get(wt_id).cloned() else {
            continue;
        };
        let source_hardlinked = attribution
            .artifacts_by_worktree
            .get(wt_id)
            .and_then(|rows| rows.iter().find(|r| r.kind == ArtifactKind::Source))
            .map(|r| r.hardlinked)
            .unwrap_or(true);
        if source_hardlinked {
            if trace {
                eprintln!("[trace]   source relist skipped ({wt_id}): unit has hardlinks");
            }
            worktrees_to_rewalk.insert(wt_id.clone());
            continue;
        }
        let artifact_rels: HashSet<String> = attribution
            .artifacts_by_worktree
            .get(wt_id)
            .map(|rows| {
                rows.iter()
                    .filter(|r| !r.kind.is_worktree_remainder())
                    .map(|r| rel_path_string(&root, &r.path))
                    .collect()
            })
            .unwrap_or_default();
        match relist_source_dirs(
            &root,
            wt_id,
            changed,
            &artifact_rels,
            &mut attribution.dirs,
            &mut attribution.files,
            large_file_min_bytes,
        ) {
            Some((new_source_local, removed_artifact_rels)) => {
                relisted += 1;
                if let Some(rows) = attribution.artifacts_by_worktree.get_mut(wt_id) {
                    // Artifact roots that vanished with a deleted directory.
                    rows.retain(|r| {
                        let rel = rel_path_string(&root, &r.path);
                        if !r.kind.is_worktree_remainder() && removed_artifact_rels.contains(&rel) {
                            total_delta -= r.bytes as i64;
                            false
                        } else {
                            true
                        }
                    });
                    if let Some(src) = rows.iter_mut().find(|r| r.kind == ArtifactKind::Source) {
                        let old_local = if src.local_bytes == 0 {
                            src.bytes
                        } else {
                            src.local_bytes
                        };
                        let delta = new_source_local as i64 - old_local as i64;
                        src.bytes = (src.bytes as i64 + delta).max(0) as u64;
                        src.local_bytes = new_source_local;
                        src.observed_at = observed_at;
                        total_delta += delta;
                    }
                }
            }
            None => {
                if trace {
                    eprintln!(
                        "[trace]   source relist fell back ({wt_id}): a changed directory is new to the store"
                    );
                }
                worktrees_to_rewalk.insert(wt_id.clone());
            }
        }
    }
    if trace {
        eprintln!(
            "[trace] incremental: relist {} worktrees' source dirs in place ({} fell back to a re-walk): {:?}",
            relisted,
            source_dirs_to_relist.len() - relisted,
            t_relist.elapsed()
        );
    }

    // New checkouts/worktrees discovered under any scan root.
    let t_scan = std::time::Instant::now();
    for scan_root in &discovery_scan_roots {
        {
            let found = crate::walk::discover_shallow(scan_root);
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
    if trace {
        eprintln!(
            "[trace] incremental: shallow discovery at {} changed dirs: {:?}",
            discovery_scan_roots.len(),
            t_scan.elapsed()
        );
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

    // Resize individual artifact roots.
    let t_resize = std::time::Instant::now();
    for (root_path, (worktree_id, kind, changed_here)) in &artifact_roots_to_resize {
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
            let wt_root = worktree_root.get(worktree_id).cloned().unwrap_or_default();
            let rel_root = rel_path_string(&wt_root, root_path);
            attribution
                .dirs
                .retain(|d| !(d.worktree_id == *worktree_id && under(&d.rel_path, &rel_root)));
            continue;
        }
        let Some(wt_root) = worktree_root.get(worktree_id).cloned() else {
            continue;
        };
        let rel_root = rel_path_string(&wt_root, root_path);
        // Cheap path: the store holds this unit's interior directory
        // rows, so only the directories FSEvents named are re-listed and
        // the unit's total is re-aggregated from the rows.
        let new_local: u64;
        let new_mtime: u64;
        let measured_hardlinked: bool;
        // Update path allocations without retaining an inode inventory.
        // Hardlinked units keep their last unique-byte measurement as stale.
        let hardlinked = attribution
            .artifacts_by_worktree
            .get(worktree_id)
            .and_then(|rows| rows.iter().find(|r| &r.path == root_path))
            .map(|r| r.hardlinked)
            .unwrap_or(true);
        let has_interior = attribution
            .dirs
            .iter()
            .any(|d| d.worktree_id == *worktree_id && d.rel_path == rel_root);
        if has_interior
            && let Some((local, mtime, saw_hardlink)) = resize_interior(
                &wt_root,
                worktree_id,
                &rel_root,
                changed_here,
                &mut attribution.dirs,
            )
        {
            if hardlinked || saw_hardlink {
                if let Some(existing) = attribution
                    .artifacts_by_worktree
                    .get_mut(worktree_id)
                    .and_then(|rows| rows.iter_mut().find(|r| &r.path == root_path))
                {
                    // Allocation rollups are current. Unique-byte charges stay
                    // at their last measurement until a full reconciliation.
                    existing.dedup_stale = true;
                    existing.hardlinked = true;
                    existing.mtime_max = existing.mtime_max.max(mtime);
                    existing.observed_at = observed_at;
                }
                n_interior += 1;
                continue;
            }
            new_local = local;
            new_mtime = mtime;
            measured_hardlinked = false;
            n_interior += 1;
        } else {
            n_whole += 1;
            let (row, dirs) = crate::walk::resize_artifact_with_dirs(
                root_path,
                kind.clone(),
                observed_at,
                Some((worktree_id, &wt_root)),
            );
            attribution
                .dirs
                .retain(|d| !(d.worktree_id == *worktree_id && under(&d.rel_path, &rel_root)));
            attribution.dirs.extend(dirs);
            new_local = row.local_bytes.max(row.bytes);
            new_mtime = row.mtime_max;
            measured_hardlinked = row.hardlinked;
        }
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
                let delta = new_local as i64 - old_local as i64;
                existing.bytes = (existing.bytes as i64 + delta).max(0) as u64;
                existing.local_bytes = new_local;
                existing.dedup_stale = false;
                existing.hardlinked = measured_hardlinked;
                existing.mtime_max = existing.mtime_max.max(new_mtime);
                existing.observed_at = observed_at;
                existing.source = crate::report::Source::new("filesystem.fsevents");
                total_delta += delta;
            } else {
                let (row, _) = crate::walk::resize_artifact_with_dirs(
                    root_path,
                    kind.clone(),
                    observed_at,
                    Some((worktree_id, &wt_root)),
                );
                total_delta += row.bytes as i64;
                rows.push(row);
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

    if trace {
        eprintln!(
            "[trace] incremental: resize {} artifact roots: {:?}",
            artifact_roots_to_resize.len(),
            t_resize.elapsed()
        );
    }
    let t_rewalk = std::time::Instant::now();
    for worktree_id in &worktrees_to_rewalk {
        let Some(root) = worktree_root.get(worktree_id) else {
            continue;
        };
        // Every stored artifact root in this worktree that no changed
        // directory touches is carried forward as-is; the walk re-sizes
        // only the implicated ones (a touch inside `target/` re-sizes
        // `target/`, not the six other artifacts next to it).
        let carry: HashMap<PathBuf, ArtifactRow> = attribution
            .artifacts_by_worktree
            .get(worktree_id)
            .map(|rows| {
                rows.iter()
                    .filter(|r| !r.kind.is_worktree_remainder())
                    .filter(|r| {
                        !changed_dirs
                            .iter()
                            .any(|c| c == &r.path || c.starts_with(&r.path))
                    })
                    .map(|r| (r.path.clone(), r.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let carried_rels: Vec<String> = carry.keys().map(|p| rel_path_string(root, p)).collect();
        let fresh = crate::walk::attribute_one_worktree(
            root,
            &all_worktree_refs,
            observed_at,
            large_file_min_bytes,
            carry,
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
        // Rows under a carried-forward artifact are still current: the
        // walk did not enter those trees, so it produced no rows for them.
        attribution.dirs.retain(|d| {
            &d.worktree_id != worktree_id || carried_rels.iter().any(|r| under(&d.rel_path, r))
        });
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

    if trace {
        eprintln!(
            "[trace] incremental: re-walk {} worktrees: {:?}",
            worktrees_to_rewalk.len(),
            t_rewalk.elapsed()
        );
    }
    split_remainder(&discovered, &mut attribution);
    Ok(TrackedWalk {
        discovered,
        attribution,
        mode: "incremental",
        // Filled in by the caller, which is the only place that still
        // holds the replay's unfiltered answer.
        event_window: None,
        changed_paths: Some(changed_dirs.to_vec()),
        reason: "incremental",
        changed_dirs: changed_dirs.len(),
        rewalked: Some(
            worktrees_to_rewalk
                .iter()
                .cloned()
                .chain(
                    artifact_roots_to_resize
                        .values()
                        .map(|(id, _, _)| id.clone()),
                )
                .collect(),
        ),
        in_place: (n_interior, n_whole, relisted, worktrees_to_rewalk.len()),
        // Set by the caller (`stage_tracked_with_source`), which has both
        // the previous topology and this result's `discovered` in hand.
        unconfirmed_worktree_ids: Vec::new(),
    })
}

// ---------------------------------------------------------------------
// #43: external/shared storage units -- a new key family in this same
// current + reverse-delta store, not a new store. Identity is
// `(detector_id, category, device, canonical_path)`: independent of any
// project/worktree, unlike the artifact rows above. Kept scope-wide
// (directly under `swamp_dir`, not per-volume) because an external
// unit's device need not match any scan root's device.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredExternalRow {
    pub(crate) detector_id: String,
    pub(crate) category: String,
    pub(crate) device: u64,
    pub(crate) path: String,
    pub(crate) bytes: u64,
    pub(crate) hardlinked: bool,
    pub(crate) present: bool,
    pub(crate) observed_at: u64,
    pub(crate) regrowth_count: u32,
}

pub(crate) fn external_row_key(
    detector_id: &str,
    category: &str,
    device: u64,
    path: &str,
) -> String {
    format!("{detector_id}\u{1}{category}\u{1}{device}\u{1}{path}")
}

fn external_dir(swamp_dir: &Path) -> PathBuf {
    swamp_dir.join("external")
}
fn external_current_path(dir: &Path) -> PathBuf {
    dir.join("current.parquet")
}
fn external_deltas_dir(dir: &Path) -> PathBuf {
    dir.join("deltas")
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
    write_parquet_atomic(path, schema, &batch, ARTIFACT_ZSTD_LEVEL)
}

fn read_external_rows(path: &Path) -> Result<Vec<StoredExternalRow>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .and_then(|b| b.build())
        .with_context(|| {
            format!(
                "read {} (delete it to rebuild this store from a full walk)",
                path.display()
            )
        })?;
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

// ---------------------------------------------------------------------
// external/folded.parquet -- the measurement the next pass may reuse
// ---------------------------------------------------------------------

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

fn folded_path(swamp_dir: &Path) -> PathBuf {
    external_dir(swamp_dir).join("folded.parquet")
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

fn write_folded_rows(path: &Path, rows: &[FoldedRow]) -> Result<()> {
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
    write_parquet_atomic(path, schema, &batch, ARTIFACT_ZSTD_LEVEL)
}

fn read_folded_rows(path: &Path) -> Result<Vec<FoldedRow>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .and_then(|b| b.build())
        .with_context(|| format!("read {}", path.display()))?;
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

/// Every stored folded row for `unit_path`, root row first. Empty when
/// nothing is stored, the store is unreadable, or the table is corrupt:
/// a cache that cannot be read is a cache miss, never an error.
pub fn folded_rows_for(swamp_dir: &Path, unit_path: &str) -> Vec<FoldedRow> {
    let mut rows: Vec<FoldedRow> = read_folded_rows(&folded_path(swamp_dir))
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.unit_path == unit_path)
        .collect();
    rows.sort_by_key(|a| a.rel_dir.len());
    rows
}

/// Replaces the stored folded rows for `unit_path` with `rows`, leaving
/// every other unit's rows alone. A unit whose rows are dropped simply
/// re-measures next pass.
/// Re-stamps the stored folded rows of `unit_paths` as verified at
/// `observed_at`, in one read and one write for the whole set.
///
/// A unit whose measurement was *reused* this pass writes no rows -- the
/// point of the reuse is that there is nothing new to write -- but its
/// rows really were re-verified, by this pass's event window. Leaving
/// their `observed_at` at the value the last full measurement wrote
/// would make every second pass a miss: the next window starts where
/// this pass ended, and rows stamped before it cannot be vouched for
/// (`crate::fs_events::EventCoverage::unchanged_since`).
pub fn touch_folded_rows(swamp_dir: &Path, unit_paths: &[String], observed_at: u64) -> Result<()> {
    if unit_paths.is_empty() {
        return Ok(());
    }
    let dir = external_dir(swamp_dir);
    fs::create_dir_all(&dir)?;
    let path = folded_path(swamp_dir);
    let wanted: std::collections::HashSet<&str> = unit_paths.iter().map(String::as_str).collect();
    let mut all: Vec<FoldedRow> = read_folded_rows(&path).unwrap_or_default();
    let mut touched = false;
    for row in all.iter_mut() {
        if wanted.contains(row.unit_path.as_str()) && row.observed_at != observed_at {
            row.observed_at = observed_at;
            touched = true;
        }
    }
    if !touched {
        return Ok(());
    }
    write_folded_rows(&path, &all)
}

pub fn store_folded_rows(swamp_dir: &Path, unit_path: &str, rows: &[FoldedRow]) -> Result<()> {
    let dir = external_dir(swamp_dir);
    fs::create_dir_all(&dir)?;
    let path = folded_path(swamp_dir);
    let mut all: Vec<FoldedRow> = read_folded_rows(&path)
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.unit_path != unit_path)
        .collect();
    all.extend(rows.iter().cloned());
    write_folded_rows(&path, &all)
}

/// Which family of rows in the shared external current table an
/// observation speaks for. External/detector-level units and agent-tool
/// units share one table and one key scheme (`agent:`-prefixed
/// categories distinguish them), which is deliberate -- one store, one
/// key family, two granularities -- but it means neither observation may
/// assume a key it did not see has disappeared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyFamily {
    /// Detector-level external storage units (`crate::external`).
    External,
    /// Units *inside* a tool home (`crate::agents`).
    Agent,
}

impl KeyFamily {
    fn matches(self, category: &str) -> bool {
        let is_agent = category.starts_with("agent:");
        match self {
            Self::Agent => is_agent,
            Self::External => !is_agent,
        }
    }
}

/// What one observation pass is entitled to tombstone
/// (`.oh/guardrails/history-sweeps-are-owned.md`).
///
/// The 2026-09-21 review's `unchanged_combined_observation_must_not_invent_regrowth`
/// counterexample: external and agent discovery both swept the shared
/// current table for keys they had not seen, so each tombstoned the
/// other's rows and the next pass recorded the resurrection as regrowth
/// -- pure fiction, on an unchanged filesystem.
///
/// A row may only be marked absent when **both** hold:
///
/// * it belongs to this observation's [`KeyFamily`], and
/// * its path lies inside a root this observation actually covered
///   completely this pass.
///
/// A root that was excluded, whose detector was disabled, that was
/// missing, unreadable, or simply not part of this pass contributes no
/// covered root, so nothing under it can be tombstoned. Coverage changes
/// are not storage changes.
#[derive(Debug, Clone)]
pub struct ObservationOwnership {
    pub family: KeyFamily,
    pub covered_roots: Vec<PathBuf>,
    /// Regions that lie *inside* a covered root but outside what this
    /// pass actually observed: a nested location the user excluded, one
    /// whose detector is disabled, one outside the explicit command
    /// roots.
    ///
    /// A path-prefix window alone cannot express this, and that is
    /// precisely how a one-line `exclude` change became a storage
    /// change: the excluded child's stored row still lay under the
    /// measured parent's root, so the owned sweep tombstoned it, and
    /// removing the line again scored a regrowth. Zero bytes moved on
    /// disk (the 2026-09-22 re-review's CE4, violating both clauses of
    /// `.oh/guardrails/coverage-changes-are-not-storage-changes.md`).
    pub excluded_subtrees: Vec<PathBuf>,
}

impl ObservationOwnership {
    pub fn new(family: KeyFamily, covered_roots: Vec<PathBuf>) -> Self {
        Self {
            family,
            covered_roots,
            excluded_subtrees: Vec::new(),
        }
    }

    /// The same window with the regions this pass did *not* observe
    /// subtracted.
    pub fn excluding(mut self, excluded: Vec<PathBuf>) -> Self {
        self.excluded_subtrees = excluded;
        self
    }

    /// Whether `path` lies inside a region this observation covered
    /// *and* observed. An excluded subtree is inside the window and
    /// outside the pass, so it is not owned: unobserved is not deleted.
    pub fn covers(&self, path: &str) -> bool {
        let p = Path::new(path);
        if self
            .excluded_subtrees
            .iter()
            .any(|e| p == e.as_path() || p.starts_with(e))
        {
            return false;
        }
        self.covered_roots
            .iter()
            .any(|r| p == r.as_path() || p.starts_with(r))
    }

    /// Whether this observation owns the stored row `key` (family +
    /// coverage). The tombstone loop is guarded by this and nothing else.
    fn owns(&self, key: &str) -> bool {
        let mut parts = key.split('\u{1}');
        let (_detector, Some(category), _device, Some(path)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return false;
        };
        self.family.matches(category) && self.covers(path)
    }
}

/// One external unit's observed facts for this pass, before growth
/// annotation. Mirrors [`Observed`] for artifact rows.
pub struct ObservedExternal {
    pub(crate) key: String,
    pub(crate) detector_id: String,
    pub(crate) category: String,
    pub(crate) device: u64,
    pub(crate) path: String,
    pub(crate) bytes: u64,
    pub(crate) hardlinked: bool,
}

fn external_history_index(dir: &Path, retention_days: u64, now: u64) -> Result<HistoryIndex> {
    let retention_secs = retention_days.saturating_mul(86400);
    let horizon = now.saturating_sub(retention_secs);
    let mut index: HashMap<String, Vec<(u64, u64, bool)>> = HashMap::new();
    for row in read_external_rows(&external_current_path(dir))? {
        index
            .entry(external_row_key(
                &row.detector_id,
                &row.category,
                row.device,
                &row.path,
            ))
            .or_default()
            .push((row.observed_at, row.bytes, true));
    }
    for delta_path in list_files_in(&external_deltas_dir(dir)) {
        for row in read_external_rows(&delta_path)? {
            if row.observed_at < horizon {
                continue;
            }
            index
                .entry(external_row_key(
                    &row.detector_id,
                    &row.category,
                    row.device,
                    &row.path,
                ))
                .or_default()
                .push((row.observed_at, row.bytes, true));
        }
    }
    for values in index.values_mut() {
        values.sort_by_key(|(t, _, _)| *t);
    }
    Ok(index)
}

/// Persists this pass's external-unit observations (current + reverse
/// delta, same layout as the artifact store) and returns
/// `(key -> (growth_bytes, regrowth_count))` for the caller to annotate
/// its own `ExternalUnit` rows with. `protected_keys` (mirroring `#42`'s
/// `protected_worktree_ids`): a key in this set is never tombstoned by
/// this pass even if absent from `observed` -- used when a unit's path
/// could not be confirmed gone-vs-inaccessible this pass.
pub fn observe_and_annotate_external(
    swamp_dir: &Path,
    observed: &[ObservedExternal],
    protected_keys: &HashSet<String>,
    ownership: &ObservationOwnership,
    observed_at: u64,
    retention_days: u64,
    since_secs: u64,
) -> Result<HashMap<String, (Option<i64>, u32)>> {
    let dir = external_dir(swamp_dir);
    fs::create_dir_all(&dir)?;
    let current_file = external_current_path(&dir);

    let mut current: HashMap<String, StoredExternalRow> = read_external_rows(&current_file)?
        .into_iter()
        .map(|r| {
            (
                external_row_key(&r.detector_id, &r.category, r.device, &r.path),
                r,
            )
        })
        .collect();

    let mut seen_keys: HashSet<String> = HashSet::new();
    let mut current_changed = false;
    let mut delta_rows: Vec<StoredExternalRow> = Vec::new();

    for obs in observed {
        seen_keys.insert(obs.key.clone());
        match current.get_mut(&obs.key) {
            Some(prev) => {
                let changed = prev.bytes != obs.bytes || !prev.present;
                if changed {
                    current_changed = true;
                    let regrowth_count = if !prev.present {
                        prev.regrowth_count + 1
                    } else {
                        prev.regrowth_count
                    };
                    delta_rows.push(prev.clone());
                    prev.bytes = obs.bytes;
                    prev.present = true;
                    prev.observed_at = observed_at;
                    prev.regrowth_count = regrowth_count;
                }
                if prev.hardlinked != obs.hardlinked {
                    current_changed = true;
                }
                prev.hardlinked = obs.hardlinked;
            }
            None => {
                current_changed = true;
                current.insert(
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

    // The owned sweep. `ownership.owns` is the whole guard: a key from
    // the other family, or one outside the regions this pass actually
    // covered, is left exactly as it is -- never tombstoned, so never
    // resurrected as invented regrowth on the next pass.
    for (key, row) in current.iter_mut() {
        if row.present
            && !seen_keys.contains(key)
            && !protected_keys.contains(key)
            && ownership.owns(key)
        {
            delta_rows.push(row.clone());
            row.present = false;
            row.bytes = 0;
            row.observed_at = observed_at;
            current_changed = true;
        }
    }

    let target_time = observed_at.saturating_sub(since_secs);
    let history_index = external_history_index(&dir, retention_days, observed_at)?;
    let mut annotations: HashMap<String, (Option<i64>, u32)> = HashMap::new();
    for obs in observed {
        let history = history_index.get(&obs.key).cloned().unwrap_or_default();
        let growth = growth_since(&history, obs.bytes, target_time);
        let regrowth = current.get(&obs.key).map(|r| r.regrowth_count).unwrap_or(0);
        annotations.insert(obs.key.clone(), (growth, regrowth));
    }

    if !delta_rows.is_empty() {
        let seq_path = next_seq_path(&external_deltas_dir(&dir), "delta-");
        write_external_rows(&seq_path, &delta_rows)?;
    }

    let mut current_rows: Vec<StoredExternalRow> = current.into_values().collect();
    current_rows.sort_by(|a, b| {
        (&a.detector_id, &a.category, a.device, &a.path).cmp(&(
            &b.detector_id,
            &b.category,
            b.device,
            &b.path,
        ))
    });
    if current_changed {
        write_external_rows(&current_file, &current_rows)?;
    }

    let files = list_files_in(&external_deltas_dir(&dir));
    if should_compact(&files) {
        let retention_secs = retention_days.saturating_mul(86400);
        let horizon = observed_at.saturating_sub(retention_secs);
        let mut merged: Vec<StoredExternalRow> = Vec::new();
        for path in &files {
            for row in read_external_rows(path)? {
                if row.observed_at >= horizon {
                    merged.push(row);
                }
            }
        }
        if !merged.is_empty() {
            write_external_rows(
                &next_seq_path(&external_deltas_dir(&dir), "delta-"),
                &merged,
            )?;
        }
        for path in &files {
            fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
        }
    }

    Ok(annotations)
}

/// The current stored `(bytes, regrowth_count)` for one external-unit
/// key, straight off `current.parquet`, with no history-window
/// computation -- what a caller needs to show a unit's last known value
/// when this pass could not re-measure it (access lost, not deleted).
pub fn peek_external_current(swamp_dir: &Path, key: &str) -> Result<Option<(u64, u32)>> {
    let dir = external_dir(swamp_dir);
    let current_file = external_current_path(&dir);
    if !current_file.exists() {
        return Ok(None);
    }
    for row in read_external_rows(&current_file)? {
        if external_row_key(&row.detector_id, &row.category, row.device, &row.path) == key {
            return Ok(Some((row.bytes, row.regrowth_count)));
        }
    }
    Ok(None)
}

/// Read-only counterpart to [`observe_and_annotate_external`]: annotates
/// from existing history without writing a new observation.
pub fn annotate_readonly_external(
    swamp_dir: &Path,
    keys: &[String],
    observed_at: u64,
    retention_days: u64,
    since_secs: u64,
) -> Result<HashMap<String, (Option<i64>, u32)>> {
    let dir = external_dir(swamp_dir);
    let current_file = external_current_path(&dir);
    if !current_file.exists() {
        return Ok(HashMap::new());
    }
    let current: HashMap<String, StoredExternalRow> = read_external_rows(&current_file)?
        .into_iter()
        .map(|r| {
            (
                external_row_key(&r.detector_id, &r.category, r.device, &r.path),
                r,
            )
        })
        .collect();
    let target_time = observed_at.saturating_sub(since_secs);
    let history_index = external_history_index(&dir, retention_days, observed_at)?;
    let mut out = HashMap::new();
    for key in keys {
        let history = history_index.get(key).cloned().unwrap_or_default();
        let bytes_now = current.get(key).map(|r| r.bytes).unwrap_or(0);
        let growth = growth_since(&history, bytes_now, target_time);
        let regrowth = current.get(key).map(|r| r.regrowth_count).unwrap_or(0);
        out.insert(key.clone(), (growth, regrowth));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn artifact_and_file_compaction_preserve_sources_on_publish_failure() -> anyhow::Result<()> {
        use super::*;
        for artifact in [true, false] {
            let tmp = tempfile::tempdir()?;
            let dir = tmp.path();
            let delta_dir = if artifact {
                deltas_dir(dir)
            } else {
                files_deltas_dir(dir)
            };
            for i in 0..8 {
                let path = next_seq_path(&delta_dir, "delta-");
                if artifact {
                    write_rows(
                        &path,
                        &[StoredRow {
                            project_id: "p".into(),
                            worktree_id: "w".into(),
                            kind: "BuildOutput".into(),
                            rel_path: "target".into(),
                            bytes: i,
                            local_bytes: i,
                            mtime_max: i,
                            hardlinked: false,
                            dedup_stale: false,
                            present: i % 2 == 0,
                            observed_at: i,
                            regrowth_count: i as u32,
                        }],
                    )?;
                } else {
                    write_file_rows(
                        &path,
                        &[StoredFileRow {
                            worktree_id: "w".into(),
                            rel_path: "a".into(),
                            allocated: i,
                            mod_time_min: i as i32,
                            observed_at: i,
                        }],
                        3,
                    )?;
                }
            }
            let files = list_files_in(&delta_dir);
            let contents = files
                .iter()
                .map(fs::read)
                .collect::<std::io::Result<Vec<_>>>()?;
            let blocker = next_seq_path(&delta_dir, "delta-");
            fs::create_dir(&blocker)?;
            let result = if artifact {
                compact_if_needed(dir, 30, 100)
            } else {
                compact_file_deltas_if_needed(dir, 30, 100)
            };
            assert!(result.is_err());
            for (p, b) in files.iter().zip(&contents) {
                assert_eq!(&fs::read(p)?, b);
            }
            fs::remove_dir(blocker)?; // empty directory belonging to this fixture
            if artifact {
                let expected = files
                    .iter()
                    .map(|p| read_rows(p))
                    .collect::<Result<Vec<_>>>()?
                    .concat();
                compact_if_needed(dir, 30, 100)?;
                let after = list_files_in(&delta_dir);
                assert_eq!(after.len(), 1);
                assert_eq!(read_rows(&after[0])?, expected);
            } else {
                let expected = files
                    .iter()
                    .map(|p| read_file_rows(p))
                    .collect::<Result<Vec<_>>>()?
                    .concat();
                compact_file_deltas_if_needed(dir, 30, 100)?;
                let after = list_files_in(&delta_dir);
                assert_eq!(after.len(), 1);
                assert_eq!(read_file_rows(&after[0])?, expected);
            }
        }
        Ok(())
    }
    #[test]
    fn small_delta_compaction_is_lossless_and_publication_failure_keeps_sources()
    -> anyhow::Result<()> {
        use super::*;
        for fail in [false, true] {
            let tmp = tempfile::tempdir()?;
            let dir = tmp.path();
            let mut expected = Vec::new();
            for i in 0..8 {
                let row = StoredDirRow {
                    worktree_id: "w".into(),
                    rel_path: "target/debug".into(),
                    parent_rel_path: Some("target".into()),
                    allocated_total: i * 4096,
                    own_allocated: i * 512,
                    file_count: i as u32,
                    entry_count: i as u32 + 1,
                    symlink_count: 0,
                    mod_time_min: i as i32,
                    complete: i % 2 == 0,
                    observed_at: 100 + i,
                };
                write_dir_rows(
                    &next_seq_path(&dirs_deltas_dir(dir), "delta-"),
                    std::slice::from_ref(&row),
                    3,
                )?;
                expected.push(row);
            }
            let before = list_files_in(&dirs_deltas_dir(dir));
            let bytes: u64 = before.iter().map(|p| fs::metadata(p).unwrap().len()).sum();
            let contents = before
                .iter()
                .map(fs::read)
                .collect::<std::io::Result<Vec<_>>>()?;
            if fail {
                fs::create_dir(next_seq_path(&dirs_deltas_dir(dir), "delta-"))?;
                assert!(compact_dir_deltas_if_needed(dir, 30, 200).is_err());
                for (path, content) in before.iter().zip(contents) {
                    assert_eq!(fs::read(path)?, content);
                }
            } else {
                compact_dir_deltas_if_needed(dir, 30, 200)?;
                let after = list_files_in(&dirs_deltas_dir(dir));
                assert_eq!(after.len(), 1);
                let restored = read_dir_rows(&after[0])?;
                assert_eq!(
                    expected, restored,
                    "all metadata, coverage and timestamps must survive"
                );
                assert!(fs::metadata(&after[0])?.len() < bytes / 2);
            }
        }
        Ok(())
    }

    #[test]
    #[ignore = "read-only real-store delta packing comparison; set SWAMP_ENCODING_INPUT"]
    fn compare_real_delta_packing() -> anyhow::Result<()> {
        use super::*;
        let source =
            PathBuf::from(std::env::var_os("SWAMP_ENCODING_INPUT").context("input required")?);
        let tmp = tempfile::tempdir()?;
        for volume in fs::read_dir(source)? {
            let volume = volume?;
            if !volume.file_type()?.is_dir() {
                continue;
            }
            let dest = tmp.path().join(volume.file_name());
            fs::create_dir(&dest)?;
            for name in ["deltas", "dirs_deltas", "files_deltas"] {
                let inputs = list_files_in(&volume.path().join(name));
                if inputs.is_empty() {
                    continue;
                }
                fs::create_dir_all(dest.join(name))?;
                let before: u64 = inputs.iter().map(|p| fs::metadata(p).unwrap().len()).sum();
                for p in &inputs {
                    fs::copy(p, dest.join(name).join(p.file_name().unwrap()))?;
                }
                match name {
                    "deltas" => {
                        let mut expected = Vec::new();
                        for p in &inputs {
                            expected.extend(read_rows(p)?);
                        }
                        expected.sort_by_key(|r| format!("{:?}", r));
                        compact_if_needed(&dest, u64::MAX, 0)?;
                        let mut actual = Vec::new();
                        for p in list_files_in(&dest.join(name)) {
                            actual.extend(read_rows(&p)?);
                        }
                        actual.sort_by_key(|r| format!("{:?}", r));
                        assert_eq!(expected, actual);
                    }
                    "dirs_deltas" => {
                        let mut expected = Vec::new();
                        for p in &inputs {
                            expected.extend(read_dir_rows(p)?);
                        }
                        expected.sort_by_key(|r| format!("{:?}", r));
                        compact_dir_deltas_if_needed(&dest, u64::MAX, 0)?;
                        let mut actual = Vec::new();
                        for p in list_files_in(&dest.join(name)) {
                            actual.extend(read_dir_rows(&p)?);
                        }
                        actual.sort_by_key(|r| format!("{:?}", r));
                        assert_eq!(expected, actual);
                    }
                    _ => {
                        let mut expected = Vec::new();
                        for p in &inputs {
                            expected.extend(read_file_rows(p)?);
                        }
                        expected.sort_by_key(|r| format!("{:?}", r));
                        compact_file_deltas_if_needed(&dest, u64::MAX, 0)?;
                        let mut actual = Vec::new();
                        for p in list_files_in(&dest.join(name)) {
                            actual.extend(read_file_rows(&p)?);
                        }
                        actual.sort_by_key(|r| format!("{:?}", r));
                        assert_eq!(expected, actual);
                    }
                }
                let after: u64 = list_files_in(&dest.join(name))
                    .iter()
                    .map(|p| fs::metadata(p).unwrap().len())
                    .sum();
                println!(
                    "{}/{name}: before={before} after={after}",
                    volume.file_name().to_string_lossy()
                );
            }
        }
        Ok(())
    }
    #[test]
    #[ignore = "read-only real-store comparison; set SWAMP_ENCODING_INPUT"]
    fn compare_real_store_encoding() -> anyhow::Result<()> {
        use super::*;
        fn collect(path: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    collect(&entry.path(), out)?;
                } else if entry.path().extension().is_some_and(|x| x == "parquet") {
                    out.push(entry.path());
                }
            }
            Ok(())
        }
        let source = PathBuf::from(
            std::env::var_os("SWAMP_ENCODING_INPUT").context("SWAMP_ENCODING_INPUT required")?,
        );
        let tmp = tempfile::tempdir()?;
        let mut files = Vec::new();
        collect(&source, &mut files)?;
        let (mut old_total, mut new_total) = (0, 0);
        for (i, path) in files.iter().enumerate() {
            let reader = ParquetRecordBatchReaderBuilder::try_new(File::open(path)?)?
                .with_batch_size(1_000_000)
                .build()?;
            let batches = reader.collect::<std::result::Result<Vec<_>, _>>()?;
            if batches.is_empty() {
                continue;
            }
            let schema = batches[0].schema();
            let out = tmp.path().join(format!("{i}.parquet"));
            let level = if path
                .parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("delta")
            {
                3
            } else {
                9
            };
            write_parquet_batches_atomic(&out, schema, batches.iter().cloned().map(Ok), level)?;
            let restored = ParquetRecordBatchReaderBuilder::try_new(File::open(&out)?)?
                .with_batch_size(1_000_000)
                .build()?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            assert_eq!(batches, restored, "{}", path.display());
            let old = fs::metadata(path)?.len();
            let new = fs::metadata(&out)?.len();
            old_total += old;
            new_total += new;
            println!(
                "{} old={old} new={new}",
                path.strip_prefix(&source)?.display()
            );
        }
        println!("total old={old_total} new={new_total}");
        Ok(())
    }

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

    #[test]
    fn checked_config_reads_the_scan_table() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("config.toml"),
            "[scan]\ndefaults = false\ninclude = [\"~/code\"]\nexclude = [\"~/code/scratch\"]\ndisabled_detectors = [\"homebrew\"]\n",
        )
        .unwrap();
        let cfg = load_config_checked(tmp.path()).expect("valid config");
        assert!(!cfg.scan.defaults);
        assert_eq!(cfg.scan.include, vec!["~/code".to_string()]);
        assert_eq!(cfg.scan.exclude, vec!["~/code/scratch".to_string()]);
        assert_eq!(cfg.scan.disabled_detectors, vec!["homebrew".to_string()]);
    }

    /// #41's core requirement: invalid explicit scope config must fail
    /// visibly, never silently broaden to the all-defaults scope. This
    /// is the shortcut the acceptance criteria calls out by name --
    /// falling back to `GrowthConfig::default()` on a parse error would
    /// make a typo in `[scan]` silently re-enable everything.
    #[test]
    fn invalid_scan_table_fails_visibly_instead_of_broadening_scope() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("config.toml"),
            "[scan]\ndefaults = \"yes\"\n", // wrong type: must be a bool
        )
        .unwrap();
        let err =
            load_config_checked(tmp.path()).expect_err("wrong-typed defaults must be rejected");
        assert!(
            err.to_string().contains("config.toml") || format!("{err:#}").contains("config.toml"),
            "error should name the offending file: {err:#}"
        );
        // The infallible convenience wrapper used deep in the report
        // pipeline still falls back to scalar defaults (unaffected
        // pipeline behavior); only scope-resolving call sites are
        // required to treat this as fatal (see `crates/cli/src/main.rs`'s
        // `resolve_scope`).
        assert_eq!(
            load_config(tmp.path()).scan,
            crate::scope::ScanConfig::default()
        );
    }

    #[test]
    fn malformed_toml_syntax_fails_visibly() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("config.toml"), "this is not [ valid toml\n").unwrap();
        assert!(load_config_checked(tmp.path()).is_err());
    }

    use crate::entities::Confidence;
    use crate::report::{ArtifactKind, ArtifactRow, ProjectRow, Source, WorktreeKind, WorktreeRow};
    use std::path::PathBuf;

    fn one_artifact_project(worktree_root: &Path, bytes: u64) -> ProjectRow {
        ProjectRow {
            project_id: "proj-1".to_string(),
            name: "proj".to_string(),
            remote: None,
            ecosystems: Vec::new(),
            worktrees: vec![WorktreeRow {
                worktree_id: "wt-1".to_string(),
                path: worktree_root.to_path_buf(),
                kind: WorktreeKind::Main,
                artifacts: vec![ArtifactRow {
                    kind: ArtifactKind::DependencyTree,
                    path: worktree_root.join("node_modules"),
                    bytes,
                    mtime_max: 0,
                    ecosystem: None,
                    hardlinked: false,
                    dedup_stale: false,
                    local_bytes: 0,
                    allocated_bytes: None,
                    allocated_growth_bytes: None,
                    track: None,
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
                    evidence: Vec::new(),
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
    fn stale_unique_measurements_are_gaps_until_reconciled() {
        let tmp = tempfile::tempdir().unwrap();
        let root = PathBuf::from("/repo");
        let mut projects = vec![one_artifact_project(&root, 1000)];
        observe_and_annotate(
            tmp.path(),
            1,
            &mut projects,
            1000,
            30,
            1000,
            &HashSet::new(),
        )
        .unwrap();
        projects[0].worktrees[0].artifacts[0].dedup_stale = true;
        observe_and_annotate(
            tmp.path(),
            1,
            &mut projects,
            2000,
            30,
            1000,
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(artifact_row(&projects).growth_bytes, None);
        let dir = volume_dir(tmp.path(), 1);
        assert!(read_rows(&current_path(&dir)).unwrap()[0].dedup_stale);
        let (_, totals) = history_series(&dir, 1000, 2, 2000);
        assert_eq!(totals, vec![Some(1000), None]);
        projects[0].worktrees[0].artifacts[0].dedup_stale = false;
        projects[0].worktrees[0].artifacts[0].bytes = 2000;
        observe_and_annotate(
            tmp.path(),
            1,
            &mut projects,
            3000,
            30,
            1000,
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(
            artifact_row(&projects).growth_bytes,
            None,
            "baseline at 2000 was stale"
        );
        let (_, totals) = history_series(&dir, 2000, 3, 3000);
        assert_eq!(totals, vec![Some(1000), None, Some(2000)]);
    }

    #[test]
    fn first_observation_has_no_growth() {
        let tmp = tempfile::tempdir().unwrap();
        let root = PathBuf::from("/repo");
        let mut projects = vec![one_artifact_project(&root, 1_000_000)];
        observe_and_annotate(
            tmp.path(),
            1,
            &mut projects,
            1_000,
            30,
            3600,
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(artifact_row(&projects).growth_bytes, None);
        assert_eq!(artifact_row(&projects).regrowth_count, 0);
    }

    #[test]
    fn second_observation_reports_growth_since_first() {
        let tmp = tempfile::tempdir().unwrap();
        let root = PathBuf::from("/repo");

        let mut first = vec![one_artifact_project(&root, 1_000_000)];
        observe_and_annotate(tmp.path(), 1, &mut first, 1_000, 30, 3600, &HashSet::new()).unwrap();

        let mut second = vec![one_artifact_project(&root, 4_000_000)];
        observe_and_annotate(tmp.path(), 1, &mut second, 2_000, 30, 3600, &HashSet::new()).unwrap();

        assert_eq!(artifact_row(&second).growth_bytes, Some(3_000_000));
    }

    #[test]
    fn unchanged_observation_appends_no_delta_file() {
        let tmp = tempfile::tempdir().unwrap();
        let root = PathBuf::from("/repo");

        let mut first = vec![one_artifact_project(&root, 1_000_000)];
        observe_and_annotate(tmp.path(), 1, &mut first, 1_000, 30, 3600, &HashSet::new()).unwrap();
        let dir = volume_dir(tmp.path(), 1);
        let after_first = list_delta_files(&dir).len();
        assert_eq!(
            after_first, 0,
            "a brand-new row has no prior state to diff against, so the very \
             first observation must not fabricate a delta either"
        );

        let mut second = vec![one_artifact_project(&root, 1_000_000)];
        observe_and_annotate(tmp.path(), 1, &mut second, 2_000, 30, 3600, &HashSet::new()).unwrap();
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
        observe_and_annotate(
            tmp.path(),
            1,
            &mut present,
            1_000,
            30,
            3600,
            &HashSet::new(),
        )
        .unwrap();

        // target/ deleted: no artifacts observed this pass at all.
        let mut absent: Vec<ProjectRow> = vec![ProjectRow {
            project_id: "proj-1".to_string(),
            name: "proj".to_string(),
            remote: None,
            ecosystems: Vec::new(),
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
        observe_and_annotate(tmp.path(), 1, &mut absent, 2_000, 30, 3600, &HashSet::new()).unwrap();

        // target/ recreated.
        let mut recreated = vec![one_artifact_project(&root, 500_000)];
        observe_and_annotate(
            tmp.path(),
            1,
            &mut recreated,
            3_000,
            30,
            3600,
            &HashSet::new(),
        )
        .unwrap();

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
        observe_and_annotate(tmp.path(), 1, &mut obs1, 1_000, 30, 5_000, &HashSet::new()).unwrap();

        let mut obs2 = vec![one_artifact_project(&root, original_bytes + 200_000_000)];
        observe_and_annotate(tmp.path(), 1, &mut obs2, 2_000, 30, 5_000, &HashSet::new()).unwrap();

        // Shrunk back to exactly the original size.
        let mut obs3 = vec![one_artifact_project(&root, original_bytes)];
        observe_and_annotate(tmp.path(), 1, &mut obs3, 3_000, 30, 5_000, &HashSet::new()).unwrap();

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
        observe_and_annotate(tmp.path(), 1, &mut obs1, 1_000, 30, 2_000, &HashSet::new()).unwrap();

        let mut obs2 = vec![one_artifact_project(&root, 2_000_000)];
        observe_and_annotate(tmp.path(), 1, &mut obs2, 2_000, 30, 2_000, &HashSet::new()).unwrap();

        // since_secs=2_000 at observed_at=3_000 targets time 1_000 --
        // exactly obs1's timestamp -- so the baseline must be obs1's
        // 1_000_000 bytes, not 0.
        let mut obs3 = vec![one_artifact_project(&root, 5_000_000)];
        observe_and_annotate(tmp.path(), 1, &mut obs3, 3_000, 30, 2_000, &HashSet::new()).unwrap();

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
            observe_and_annotate(
                tmp.path(),
                1,
                &mut obs,
                1_000 + i,
                30,
                3600,
                &HashSet::new(),
            )
            .unwrap();
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
        observe_and_annotate(
            tmp.path(),
            1,
            &mut projects,
            1_000,
            30,
            3600,
            &HashSet::new(),
        )
        .unwrap();

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
