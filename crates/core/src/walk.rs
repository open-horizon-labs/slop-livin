//! Parallel discovery and attribution over a bounded thread pool.
//!
//! `git::discover` and `attribution::attribute` are both plain recursive
//! walks: one directory at a time, one thread, waiting on each `lstat`
//! and `read_dir` before starting the next. On a large tree that serial
//! syscall latency, not CPU, is almost the entire wall time (measured:
//! ~45% CPU on a 15 s walk). This module re-implements each walk's exact
//! predicates and stop conditions as jobs on a bounded worker pool, so
//! many directories are in flight at once, while leaving `git::discover`
//! and `attribution::attribute` themselves untouched for their existing
//! unit tests and as the reference semantics this module must match.
//!
//! The two walks use *different* stop conditions on purpose (discovery
//! only stops at [`STOP_DIRS`] plus `.git`; attribution stops at every
//! [`classify`]-matched name, a much longer list), so a project can be
//! discovered arbitrarily deep inside a directory — `vendor`, `.venv`,
//! `Pods` — that attribution nonetheless sizes as a single opaque unit
//! without descending into it. Fusing the two walks into one pass would
//! collapse that distinction; the two are kept as independent parallel
//! passes here to preserve it exactly.

use crate::attribution::{AttributionResult, allocated_bytes, classify, is_shared_cache_name};
use crate::entities::{Confidence, id_for};
use crate::git::{DiscoveredWorktree, classify_git_file, classify_main_checkout};
use crate::report::{
    ArtifactKind, ArtifactRow, DirRollup, FileRow, Source, UnownedReason, UnownedRow,
};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// Same boundary `git::discover` stops recursion at: `.git` itself, plus
/// artifact roots that would otherwise be misread as nested project
/// roots. Duplicated here (rather than making `git::STOP_DIRS` `pub`)
/// because it is part of the exact-match contract this module documents,
/// not an implementation detail to import.
const STOP_DIRS: &[&str] = &["node_modules", "target", "dist", "build"];

/// A generic bounded job queue: `outstanding` counts every job pushed but
/// not yet finished (including one still being processed by a worker),
/// so the pool is done exactly when the queue is empty and `outstanding`
/// is zero.
struct Pool<J> {
    queue: Mutex<VecDeque<J>>,
    cv: Condvar,
    outstanding: AtomicUsize,
}

impl<J: Send> Pool<J> {
    fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            cv: Condvar::new(),
            outstanding: AtomicUsize::new(0),
        }
    }

    fn push(&self, job: J) {
        self.outstanding.fetch_add(1, Ordering::SeqCst);
        self.queue.lock().unwrap().push_back(job);
        self.cv.notify_one();
    }

    fn finish_one(&self) {
        if self.outstanding.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.cv.notify_all();
        }
    }

    fn next(&self) -> Option<J> {
        let mut q = self.queue.lock().unwrap();
        loop {
            if let Some(job) = q.pop_front() {
                return Some(job);
            }
            if self.outstanding.load(Ordering::SeqCst) == 0 {
                return None;
            }
            q = self.cv.wait(q).unwrap();
        }
    }

    /// Runs `process` on a bounded pool of worker threads until the queue
    /// drains and every in-flight job has finished. `process` is
    /// responsible for calling `pool.push` for follow-on work and must
    /// not call `finish_one` itself.
    fn drain(self: &Arc<Self>, workers: usize, process: impl Fn(J) + Sync) {
        std::thread::scope(|scope| {
            for _ in 0..workers {
                let pool = Arc::clone(self);
                let process = &process;
                scope.spawn(move || {
                    while let Some(job) = pool.next() {
                        process(job);
                        pool.finish_one();
                    }
                });
            }
        });
    }
}

/// Worker count: directory traversal here is latency-bound on metadata
/// syscalls, not CPU, so running more workers than cores keeps more
/// lookups in flight. Doubling cores was measured against 1x/4x/8x/16x on
/// a 38 GB, ~330-top-level-directory tree and gave the best wall time.
fn worker_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        * 2
}

// ---------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------

/// Parallel equivalent of `git::discover`: same stop conditions
/// (`STOP_DIRS` plus `.git`), same per-directory `.git` identity
/// resolution, same device/symlink guards. Order of the returned rows is
/// unspecified (workers race), which is fine: callers group by
/// `project_id`/`worktree_id`, not position.
pub fn discover_parallel(root: &Path) -> Result<Vec<DiscoveredWorktree>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let device = fs::symlink_metadata(root)
        .with_context(|| format!("stat {}", root.display()))?
        .dev();

    let discovered: Mutex<Vec<DiscoveredWorktree>> = Mutex::new(Vec::new());
    let pool: Arc<Pool<PathBuf>> = Arc::new(Pool::new());
    pool.push(root.to_path_buf());

    pool.drain(worker_count(), |path| {
        discover_one(&path, device, &pool, &discovered);
    });

    Ok(discovered.into_inner().unwrap())
}

fn discover_one(
    dir: &Path,
    device: u64,
    pool: &Pool<PathBuf>,
    discovered: &Mutex<Vec<DiscoveredWorktree>>,
) {
    let Ok(meta) = fs::symlink_metadata(dir) else {
        return;
    };
    if meta.dev() != device || meta.file_type().is_symlink() || !meta.is_dir() {
        return;
    }

    let git_path = dir.join(".git");
    if let Ok(git_meta) = fs::symlink_metadata(&git_path) {
        let dw = if git_meta.is_dir() {
            classify_main_checkout(dir, &git_path)
        } else if git_meta.is_file() {
            classify_git_file(dir, &git_path)
        } else {
            None
        };
        if let Some(dw) = dw {
            discovered.lock().unwrap().push(dw);
        }
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == ".git" || STOP_DIRS.contains(&name.as_ref()) {
            continue;
        }
        pool.push(entry.path());
    }
}

// ---------------------------------------------------------------------
// Attribution
// ---------------------------------------------------------------------

/// A worktree known to the attribution pass: its path (for nearest-match)
/// and the id it should attach rows to. Mirrors
/// `attribution::KnownWorktree`.
struct KnownWorktree {
    path: PathBuf,
    worktree_id: String,
}

/// Finds the id of the worktree whose path is the longest prefix of
/// `path` (the nearest containing checkout/worktree), if any. Mirrors
/// `attribution::nearest_worktree`.
fn nearest_worktree<'a>(worktrees: &'a [KnownWorktree], path: &Path) -> Option<&'a str> {
    worktrees
        .iter()
        .filter(|w| path.starts_with(&w.path))
        .max_by_key(|w| w.path.as_os_str().len())
        .map(|w| w.worktree_id.as_str())
}

/// The root path of the worktree with this id, for turning an absolute
/// path into a rel_path (`DirRollup`/`FileRow` keys are always relative,
/// same contract as `growth.rs`'s stored rows).
fn worktree_root_path<'a>(worktrees: &'a [KnownWorktree], id: &str) -> Option<&'a Path> {
    worktrees
        .iter()
        .find(|w| w.worktree_id == id)
        .map(|w| w.path.as_path())
}

fn rel_path_string(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let s = rel.display().to_string();
    if s == "." { String::new() } else { s }
}

fn parent_rel_path_of(rel_path: &str) -> Option<String> {
    if rel_path.is_empty() {
        return None;
    }
    match Path::new(rel_path).parent() {
        Some(p) => Some(p.display().to_string()),
        None => Some(String::new()),
    }
}

/// Number of shards for the hardlink-dedup set. One `lstat`'s worth of
/// work (a mutex lock + hash-set insert) happens per regular file across
/// every worker, so a single global mutex there would serialize the
/// whole walk; sharding by inode spreads that contention out.
const INODE_SHARDS: usize = 64;

struct ShardedInodeSet {
    shards: Vec<Mutex<HashSet<(u64, u64)>>>,
}

impl ShardedInodeSet {
    fn new() -> Self {
        Self {
            shards: (0..INODE_SHARDS)
                .map(|_| Mutex::new(HashSet::new()))
                .collect(),
        }
    }

    /// Returns true the first time `key` is seen (i.e. it should be
    /// counted), matching `HashSet::insert`'s return value.
    fn insert_first(&self, key: (u64, u64)) -> bool {
        let shard = &self.shards[key.1 as usize % INODE_SHARDS];
        shard.lock().unwrap().insert(key)
    }
}

/// Wait-group + accumulator for one classified artifact directory's
/// subtree, sized by its own bounded pool of `Size` jobs. `remaining`
/// starts at 1 (for the root job); every spawned child increments it
/// before being queued, and every finished job (root or child)
/// decrements it. The job that takes it to zero is the last one done and
/// emits the row — exactly one row per classified directory, regardless
/// of how many jobs sized its subtree.
struct SizeGroup {
    root_path: PathBuf,
    kind: ArtifactKind,
    worktree: Option<String>,
    total: AtomicU64,
    remaining: AtomicUsize,
}

enum AttrJob {
    /// An unclassified directory: recurse, classifying each child by
    /// name and either sizing it as a unit or walking further.
    Walk(PathBuf),
    /// A directory inside an already-classified artifact subtree.
    Size {
        path: PathBuf,
        group: Arc<SizeGroup>,
    },
}

struct AttrShared {
    seen_inodes: ShardedInodeSet,
    artifacts_by_worktree: Mutex<HashMap<String, Vec<ArtifactRow>>>,
    source_bytes: Mutex<HashMap<String, u64>>,
    unowned: Mutex<Vec<UnownedRow>>,
    walked_total: AtomicU64,
    attributed_total: AtomicU64,
    unowned_total: AtomicU64,
    observed_at: u64,
    /// R4c: one entry per Source-tree directory, keyed by
    /// `(worktree_id, rel_path)`. Never populated for a directory inside
    /// a folded artifact (those are `Size` jobs, not `Walk` jobs) or for
    /// a directory outside every known worktree.
    dirs: Mutex<HashMap<(String, String), DirRollup>>,
    /// R4c: large-file rows (>= `large_file_min_bytes`) found directly
    /// while walking the Source tree.
    files: Mutex<Vec<FileRow>>,
    large_file_min_bytes: u64,
}

/// Parallel equivalent of `attribution::attribute`: same classification
/// table, same nearest-containing-worktree attribution (by longest path
/// prefix over the full `worktrees` list, computed once up front —
/// unlike discovery, attribution never changes what worktrees exist
/// while it runs), same hardlink dedup, same one-`Source`-row-per-worktree
/// fold at the end.
pub fn attribute_parallel(
    root: &Path,
    worktrees: &[(&Path, &str)],
    observed_at: u64,
    large_file_min_bytes: u64,
) -> AttributionResult {
    let known: Vec<KnownWorktree> = worktrees
        .iter()
        .map(|(path, id)| KnownWorktree {
            path: path.to_path_buf(),
            worktree_id: id.to_string(),
        })
        .collect();
    let known = Arc::new(known);

    let shared = Arc::new(AttrShared {
        seen_inodes: ShardedInodeSet::new(),
        artifacts_by_worktree: Mutex::new(HashMap::new()),
        source_bytes: Mutex::new(HashMap::new()),
        unowned: Mutex::new(Vec::new()),
        walked_total: AtomicU64::new(0),
        attributed_total: AtomicU64::new(0),
        unowned_total: AtomicU64::new(0),
        observed_at,
        dirs: Mutex::new(HashMap::new()),
        files: Mutex::new(Vec::new()),
        large_file_min_bytes,
    });

    let pool: Arc<Pool<AttrJob>> = Arc::new(Pool::new());
    pool.push(AttrJob::Walk(root.to_path_buf()));

    pool.drain(worker_count(), |job| match job {
        AttrJob::Walk(path) => process_walk(path, &known, &shared, &pool),
        AttrJob::Size { path, group } => process_size(path, &group, &shared, &pool),
    });

    let shared = Arc::try_unwrap(shared).unwrap_or_else(|_| unreachable!("workers joined"));
    let mut artifacts_by_worktree = shared.artifacts_by_worktree.into_inner().unwrap();
    let source_bytes = shared.source_bytes.into_inner().unwrap();

    for (worktree_id, bytes) in source_bytes {
        if bytes == 0 {
            continue;
        }
        let path = worktrees
            .iter()
            .find(|(_, id)| *id == worktree_id)
            .map(|(p, _)| p.to_path_buf())
            .unwrap_or_default();
        artifacts_by_worktree
            .entry(worktree_id)
            .or_default()
            .push(ArtifactRow {
                kind: ArtifactKind::Source,
                path,
                bytes,
                growth_bytes: None,
                regrowth_count: 0,
                observed_at,
                confidence: Confidence::High,
                source: Source::new("filesystem.walk"),
                note: None,
            });
    }

    AttributionResult {
        artifacts_by_worktree,
        unowned: shared.unowned.into_inner().unwrap(),
        walked_total: shared.walked_total.load(Ordering::Acquire),
        attributed_total: shared.attributed_total.load(Ordering::Acquire),
        unowned_total: shared.unowned_total.load(Ordering::Acquire),
        dirs: shared.dirs.into_inner().unwrap().into_values().collect(),
        files: shared.files.into_inner().unwrap(),
    }
}

/// Processes one directory in the non-classified part of the tree. Files
/// are recorded inline (one extra `lstat` for size/hardlink identity,
/// same as the serial walk); only subdirectories become new pool jobs, so
/// the number of jobs tracks the directory count rather than the much
/// larger file count. The root of the whole walk is the one path that
/// arrives here without having been through a parent's `read_dir`
/// (`is_dir`/`is_symlink` come from `DirEntry::file_type` for every other
/// call site), so it alone still needs its own `symlink_metadata` check.
fn process_walk(path: PathBuf, known: &[KnownWorktree], shared: &AttrShared, pool: &Pool<AttrJob>) {
    let Ok(meta) = fs::symlink_metadata(&path) else {
        return;
    };
    if meta.file_type().is_symlink() {
        return;
    }
    if meta.is_file() {
        record_file(&path, &meta, known, shared);
        return;
    }
    if !meta.is_dir() {
        return;
    }

    let entries = match fs::read_dir(&path) {
        Ok(e) => e,
        Err(_) => {
            shared.unowned.lock().unwrap().push(UnownedRow {
                path_or_object: path.display().to_string(),
                bytes: 0,
                reason: UnownedReason::PermissionDenied,
                shared_bytes: None,
                note: None,
                docker_kind: None,
            });
            return;
        }
    };

    // R4c: this directory's own rollup, accumulated as entries are
    // classified below. Only recorded at the end if `path` is inside a
    // known worktree's Source tree (`nearest_worktree` returns `Some`).
    let mut dir_own_allocated: u64 = 0;
    let mut dir_file_count: u32 = 0;
    let mut dir_dir_count: u32 = 0;
    let mut dir_symlink_count: u32 = 0;
    // Floor at this directory's own mtime (already lstat'd above as
    // `meta`), never epoch: a directory whose direct entries are all
    // subdirectories still has its own st_mtime as real evidence, and
    // that must win over a fabricated zero.
    let mut dir_mtime_max: i64 = meta.mtime();

    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let child_path = entry.path();
        if ft.is_symlink() {
            dir_symlink_count += 1;
            if let Ok(smeta) = fs::symlink_metadata(&child_path) {
                dir_mtime_max = dir_mtime_max.max(smeta.mtime());
            }
            continue;
        }
        if ft.is_file() {
            dir_file_count += 1;
            if let Some((mtime, bytes_opt)) = record_file_typed(&child_path, known, shared) {
                dir_mtime_max = dir_mtime_max.max(mtime);
                if let Some(bytes) = bytes_opt {
                    dir_own_allocated += bytes;
                    if bytes >= shared.large_file_min_bytes {
                        record_large_file(&child_path, bytes, mtime, known, shared);
                    }
                }
            }
        } else if ft.is_dir() {
            dir_dir_count += 1;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(kind) = classify(&name) {
                let worktree = nearest_worktree(known, &child_path).map(str::to_string);
                let group = Arc::new(SizeGroup {
                    root_path: child_path.clone(),
                    kind,
                    worktree,
                    total: AtomicU64::new(0),
                    remaining: AtomicUsize::new(1),
                });
                pool.push(AttrJob::Size {
                    path: child_path,
                    group,
                });
            } else {
                pool.push(AttrJob::Walk(child_path));
            }
        }
    }

    if let Some(worktree_id) = nearest_worktree(known, &path).map(str::to_string)
        && let Some(root) = worktree_root_path(known, &worktree_id)
    {
        let rel_path = rel_path_string(root, &path);
        let parent_rel_path = parent_rel_path_of(&rel_path);
        let mod_time_min = (dir_mtime_max / 60) as i32;
        shared.dirs.lock().unwrap().insert(
            (worktree_id.clone(), rel_path.clone()),
            DirRollup {
                worktree_id,
                rel_path,
                parent_rel_path,
                // Bottom-up aggregation into `allocated_total` happens
                // once in `report::aggregate_dir_totals`, after every
                // directory job has finished; starting equal to
                // `own_allocated` keeps this row valid even if that
                // aggregation step is skipped.
                allocated_total: dir_own_allocated,
                own_allocated: dir_own_allocated,
                file_count: dir_file_count,
                entry_count: dir_file_count + dir_dir_count + dir_symlink_count,
                symlink_count: dir_symlink_count,
                mod_time_min,
                complete: true,
                growth_bytes: None,
            },
        );
    }
}

fn record_large_file(
    path: &Path,
    bytes: u64,
    mtime_secs: i64,
    known: &[KnownWorktree],
    shared: &AttrShared,
) {
    let Some(worktree_id) = nearest_worktree(known, path) else {
        return;
    };
    let Some(root) = worktree_root_path(known, worktree_id) else {
        return;
    };
    shared.files.lock().unwrap().push(FileRow {
        worktree_id: worktree_id.to_string(),
        rel_path: rel_path_string(root, path),
        allocated: bytes,
        mod_time_min: (mtime_secs / 60) as i32,
        growth_bytes: None,
    });
}

/// Like `record_file`, but for an entry already known (from
/// `DirEntry::file_type`) to be a non-symlink file, so it does the one
/// `lstat` a regular file needs for size/hardlink identity without a
/// redundant type check first. Returns `(mtime_secs, bytes_if_newly_counted)`
/// so the caller can fold this file into its directory's rollup (mtime
/// always; bytes only when this was not a hardlink dup, matching how
/// `walked_total`/`attributed_total` already dedup).
fn record_file_typed(
    path: &Path,
    known: &[KnownWorktree],
    shared: &AttrShared,
) -> Option<(i64, Option<u64>)> {
    let meta = fs::symlink_metadata(path).ok()?;
    Some(record_file(path, &meta, known, shared))
}

fn record_file(
    path: &Path,
    meta: &fs::Metadata,
    known: &[KnownWorktree],
    shared: &AttrShared,
) -> (i64, Option<u64>) {
    let mtime = meta.mtime();
    if !shared.seen_inodes.insert_first((meta.dev(), meta.ino())) {
        return (mtime, None);
    }
    let bytes = allocated_bytes(meta);
    shared.walked_total.fetch_add(bytes, Ordering::Relaxed);
    match nearest_worktree(known, path) {
        Some(worktree_id) => {
            shared.attributed_total.fetch_add(bytes, Ordering::Relaxed);
            *shared
                .source_bytes
                .lock()
                .unwrap()
                .entry(worktree_id.to_string())
                .or_default() += bytes;
        }
        None => {
            shared.unowned_total.fetch_add(bytes, Ordering::Relaxed);
            push_unowned_file(path, bytes, shared);
        }
    }
    (mtime, Some(bytes))
}

fn push_unowned_file(path: &Path, bytes: u64, shared: &AttrShared) {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let reason = if is_shared_cache_name(name) {
        UnownedReason::SharedCache
    } else {
        UnownedReason::NoContainingRepo
    };
    shared.unowned.lock().unwrap().push(UnownedRow {
        path_or_object: path.display().to_string(),
        bytes,
        reason,
        shared_bytes: None,
        note: None,
        docker_kind: None,
    });
}

/// Sizes one directory inside a classified artifact subtree: sums
/// allocated bytes of its regular files (deduped by hardlink) into the
/// group's running total, and hands subdirectories to the pool as more
/// `Size` jobs under the same group. Read errors here are swallowed, same
/// as the serial `size_as_unit`, since a classified directory is sized as
/// a best-effort unit rather than reported as a permission gap.
fn process_size(path: PathBuf, group: &Arc<SizeGroup>, shared: &AttrShared, pool: &Pool<AttrJob>) {
    let Ok(entries) = fs::read_dir(&path) else {
        finish_size_job(group, shared);
        return;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            group.remaining.fetch_add(1, Ordering::SeqCst);
            pool.push(AttrJob::Size {
                path: entry.path(),
                group: Arc::clone(group),
            });
        } else if ft.is_file() {
            let Ok(meta) = fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if meta.file_type().is_symlink() || !meta.is_file() {
                continue;
            }
            if !shared.seen_inodes.insert_first((meta.dev(), meta.ino())) {
                continue;
            }
            let bytes = allocated_bytes(&meta);
            group.total.fetch_add(bytes, Ordering::Relaxed);
            shared.walked_total.fetch_add(bytes, Ordering::Relaxed);
            if group.worktree.is_some() {
                shared.attributed_total.fetch_add(bytes, Ordering::Relaxed);
            } else {
                shared.unowned_total.fetch_add(bytes, Ordering::Relaxed);
            }
        }
    }
    finish_size_job(group, shared);
}

fn finish_size_job(group: &Arc<SizeGroup>, shared: &AttrShared) {
    if group.remaining.fetch_sub(1, Ordering::AcqRel) != 1 {
        return;
    }
    let bytes = group.total.load(Ordering::Acquire);
    match &group.worktree {
        Some(worktree_id) => {
            shared
                .artifacts_by_worktree
                .lock()
                .unwrap()
                .entry(worktree_id.clone())
                .or_default()
                .push(ArtifactRow {
                    kind: group.kind.clone(),
                    path: group.root_path.clone(),
                    bytes,
                    growth_bytes: None,
                    regrowth_count: 0,
                    observed_at: shared.observed_at,
                    confidence: Confidence::High,
                    source: Source::new("filesystem.walk"),
                    note: None,
                });
        }
        None => {
            let name = group
                .root_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let reason = if is_shared_cache_name(name) {
                UnownedReason::SharedCache
            } else {
                UnownedReason::NoContainingRepo
            };
            shared.unowned.lock().unwrap().push(UnownedRow {
                path_or_object: group.root_path.display().to_string(),
                bytes,
                reason,
                shared_bytes: None,
                note: None,
                docker_kind: None,
            });
        }
    }
}

// ---------------------------------------------------------------------
// Combined entry point used by `report::report_with`.
// ---------------------------------------------------------------------

/// Runs the parallel discovery pass, then the parallel attribution pass
/// over the resulting worktree list, matching the two sequential calls
/// `report_with` used to make to `git::discover` and
/// `attribution::attribute`.
pub fn discover_and_attribute(
    root: &Path,
    observed_at: u64,
    large_file_min_bytes: u64,
) -> Result<(Vec<DiscoveredWorktree>, AttributionResult)> {
    let trace = std::env::var("SLOP_LIVIN_TRACE").is_ok_and(|v| v != "0" && !v.is_empty());
    let t0 = std::time::Instant::now();
    let discovered = discover_parallel(root)?;
    if trace {
        eprintln!("[trace] walk::discover_parallel: {:?}", t0.elapsed());
    }
    let worktree_ids: Vec<(PathBuf, String)> = discovered
        .iter()
        .map(|dw| (dw.path.clone(), id_for(&dw.path.display().to_string())))
        .collect();
    let worktree_refs: Vec<(&Path, &str)> = worktree_ids
        .iter()
        .map(|(p, id)| (p.as_path(), id.as_str()))
        .collect();
    let t1 = std::time::Instant::now();
    let attribution = attribute_parallel(root, &worktree_refs, observed_at, large_file_min_bytes);
    if trace {
        eprintln!("[trace] walk::attribute_parallel: {:?}", t1.elapsed());
    }
    Ok((discovered, attribution))
}
