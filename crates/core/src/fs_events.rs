//! In-process FSEvents replay: "which directories under a root changed
//! since event id N?", answered without a helper process.
//!
//! Ports the design from mole `integrate` (`cmd/fswatch`,
//! `internal/fswatch`, `docs/fsevents-helper.md`), collapsed into one
//! crate instead of a separate cgo binary: this crate is already macOS
//! product code (unlike Mole's pure-Go release binaries), so there is no
//! `CGO_ENABLED=0` constraint to protect by keeping FSEvents in a
//! sidecar. `fsevent-sys` gives raw CoreServices bindings; [`macos::replay`]
//! is the small safe wrapper around them.
//!
//! The client never treats a failed replay as an error: [`FsEventsSource::replay`]
//! always returns a [`FsEventsPlan`], and every way a replay can fail to
//! earn an incremental refresh is a named [`RefreshRefusal`] instead of an
//! `Err`. A refusal is not a bug; it is this design refusing to guess.

use crate::entities::{Confidence, FactMeta};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Every way a replay can fail to earn an incremental refresh. Two rules
/// hold across all of them: a refusal never carries a usable change list,
/// and the caller always falls back to a full walk.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RefreshRefusal {
    /// First observation of this root, or a stored id this build has never
    /// written: nothing to replay from.
    NoStoredEventId,
    /// The stored id is ahead of the stream's current id -- the volume's
    /// FSEvents history was reset and everything in between is
    /// unaccounted for.
    EventIdFromFuture,
    /// The root does not canonicalize to itself, or the root's device
    /// differs from the device the stored id was recorded against: the
    /// stored id addresses a different filesystem than the one being
    /// asked about.
    RootMismatch,
    /// No FSEvents history at all is available for this volume (no
    /// current event id, stream creation failed, or the volume is not
    /// journaled).
    FseventsdUnavailable,
    /// The replay could not be completed as a confident, complete account
    /// of what changed: `MustScanSubDirs`, `UserDropped`/`KernelDropped`,
    /// `EventIdsWrapped`, or the run loop budget expired before FSEvents
    /// reported `HistoryDone`. A prefix of the changes is not an answer.
    HelperInconclusive,
    /// The changed-directory count exceeds the configured fraction of
    /// previously known directories: re-walking that much of the tree
    /// piecemeal costs more than a full walk would.
    TooManyChanges,
    /// The previous observation of this root is too recent (within the
    /// same coarse timestamp granularity the growth store uses) for a
    /// replay to be meaningful: FSEvents' own persisted log can lag a
    /// write by longer than that gap, so a replay this soon after the
    /// baseline cannot yet distinguish "nothing changed" from "the
    /// change has not been logged yet". Decided by the caller
    /// (`growth::observe_tracked`), not by [`FsEventsSource::replay`]
    /// itself.
    TooSoon,
    /// This build has no FSEvents implementation for the current
    /// platform (non-macOS). Always refuses; there is no fallback stream
    /// to try.
    UnsupportedPlatform,
}

impl RefreshRefusal {
    /// Stable, lowercase snake_case reason code used in report coverage
    /// notes and the `observe` log line (`mode=full reason=<this>`).
    pub fn as_str(self) -> &'static str {
        match self {
            RefreshRefusal::NoStoredEventId => "no_stored_event_id",
            RefreshRefusal::EventIdFromFuture => "event_id_from_future",
            RefreshRefusal::RootMismatch => "root_mismatch",
            RefreshRefusal::FseventsdUnavailable => "fseventsd_unavailable",
            RefreshRefusal::HelperInconclusive => "helper_inconclusive",
            RefreshRefusal::TooManyChanges => "too_many_changes",
            RefreshRefusal::TooSoon => "too_soon",
            RefreshRefusal::UnsupportedPlatform => "unsupported_platform",
        }
    }
}

/// The two scalars a caller persists alongside whatever it publishes, and
/// passes back in on the next call. Mirrors mole's `fswatch.Request`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FsEventsState {
    /// The `current_event_id` from the plan that accompanied the last
    /// observation. `None` means "nothing stored", always a full refresh.
    pub event_id: Option<u64>,
    /// The device (`st_dev`) the root lived on when `event_id` was
    /// recorded. A root that now resolves to a different device makes the
    /// stored id meaningless.
    pub device: Option<u64>,
    /// `now()` (whole seconds) as of the observation that recorded
    /// `event_id`. See [`RefreshRefusal::TooSoon`].
    pub last_observed_at: Option<u64>,
    /// `ecosystem::RULES_VERSION` the stored rows were classified under.
    /// A different current version forces a full walk (missing = 0).
    #[serde(default)]
    pub rules_version: u32,
    /// This path's anchor as a *measured unit root* -- an authorized
    /// detector-resolved external cache or agent tool home
    /// (`crate::growth::replay_unit_roots`).
    ///
    /// It is a separate anchor from the three scalars above, in the same
    /// control file, because the two are advanced by different things at
    /// different times: the scalars by the folded walk of a scan root,
    /// this by the pass that measured the unit family. A path can be
    /// both (a tool home inside the configured scope), which is exactly
    /// why neither writer may rewrite the whole file -- both do a
    /// read-modify-write of their own half.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_root: Option<UnitRootCursor>,
}

/// One authorized unit root's replay anchor.
///
/// Deliberately not just a second [`FsEventsState`]: a unit root has no
/// `rules_version` of its own (nothing about it is classified by the
/// ecosystem rules) and reusing the same struct would invite a writer to
/// copy the walk's scalars into it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnitRootCursor {
    /// The `current_event_id` from the replay that accompanied the last
    /// measurement of this root.
    pub event_id: Option<u64>,
    /// The device the root lived on then. A root that now resolves to a
    /// different device makes the stored id meaningless.
    pub device: Option<u64>,
    /// `now()` (whole seconds) as of the pass that recorded `event_id`.
    /// This is the instant the next window opens from, and the floor
    /// [`EventCoverage::unchanged_since`] compares stored rows against.
    pub observed_at: Option<u64>,
}

/// One replay request: a canonical root and the state persisted from the
/// last observation of it.
#[derive(Debug, Clone)]
pub struct FsEventsRequest {
    /// Must already be canonical (absolute, symlinks resolved). FSEvents
    /// answers in canonical paths; a non-canonical root would make every
    /// path in the reply appear to sit outside the tree the caller thinks
    /// it asked about.
    pub root: PathBuf,
    pub since: FsEventsState,
}

/// The verdict: either a complete, incremental account of what changed,
/// or a refusal naming why a full walk is required instead. There is no
/// third "error" case -- a failed replay is a refusal, never an `Err`.
#[derive(Debug, Clone)]
pub struct FsEventsPlan {
    pub incremental: bool,
    pub refusal: Option<RefreshRefusal>,
    /// Meaningful only when `incremental` is true. Every reported path
    /// contributes itself and its parent directory (mirrors mole's
    /// `changeSet`), deduplicated, so a creation (visible only from the
    /// parent's listing) and a deletion (nothing left to walk) are both
    /// covered.
    pub changed_dirs: Vec<PathBuf>,
    /// The id to persist for the next call. Meaningful even on a
    /// refusal: one forced full refresh re-anchors the stream rather than
    /// condemning every later run to be full as well. `0` when the
    /// platform could not tell us one at all.
    pub current_event_id: u64,
    /// The device the root lived on for this replay, persisted alongside
    /// `current_event_id` so the next call can detect a root that moved
    /// to a different volume.
    pub device: Option<u64>,
    /// The changes came from a live stream (`watch`), not a replay of the
    /// persisted log, so the replay-lag floor (`TooSoon`) does not apply:
    /// a live event is the change, not a query that might predate it.
    pub live: bool,
}

impl FsEventsPlan {
    /// An incremental plan built from live stream batches.
    pub fn from_live(
        changed_dirs: Vec<PathBuf>,
        current_event_id: u64,
        device: Option<u64>,
    ) -> Self {
        Self {
            incremental: true,
            refusal: None,
            changed_dirs,
            current_event_id,
            device,
            live: true,
        }
    }

    fn refuse(reason: RefreshRefusal, current_event_id: u64, device: Option<u64>) -> Self {
        Self {
            incremental: false,
            refusal: Some(reason),
            changed_dirs: Vec::new(),
            current_event_id,
            device,
            live: false,
        }
    }

    fn ok(changed_dirs: Vec<PathBuf>, current_event_id: u64, device: Option<u64>) -> Self {
        Self {
            incremental: true,
            refusal: None,
            changed_dirs,
            current_event_id,
            device,
            live: false,
        }
    }

    /// The reason string for coverage notes / log lines, on either branch.
    pub fn reason_str(&self) -> &'static str {
        self.refusal.map(|r| r.as_str()).unwrap_or("incremental")
    }
}

/// The seam a real observation drives, and tests replace with canned
/// event batches so no test depends on the live `fseventsd`.
pub trait FsEventsSource: Send + Sync {
    fn replay(&self, request: &FsEventsRequest) -> FsEventsPlan;

    /// Replays several roots in one go, returning one plan per request
    /// in the same order.
    ///
    /// An implementation is free to answer them with a single stream --
    /// the macOS one opens one stream per *device*, since that is the
    /// granularity FSEvents' retained log actually has, and splits the
    /// result with [`partition_changes`] so no root ever sees another
    /// root's events. The default implementation replays them one at a
    /// time, which is what every canned test source wants and what the
    /// non-macOS stub needs.
    fn replay_roots(&self, requests: &[FsEventsRequest]) -> Vec<FsEventsPlan> {
        requests.iter().map(|r| self.replay(r)).collect()
    }
}

/// One root's trusted replay window, as `growth` hands it up to the
/// report pipeline: the replay's own (unfiltered) change list, and the
/// observation time the window opens from.
pub type TrustedWindow = (Vec<PathBuf>, u64);

/// Where `consumers::walk` leaves one root's [`TrustedWindow`] for the
/// caller that built the bus context.
pub type EventWindowSlot = std::sync::Arc<std::sync::Mutex<Option<TrustedWindow>>>;

/// One root this observation replayed successfully, and everything the
/// replay said changed underneath it.
#[derive(Debug, Clone)]
struct EventWindow {
    /// The spelling the *units* under this root are addressed in -- the
    /// one a caller passes to [`EventCoverage::unchanged_since`].
    root: PathBuf,
    /// The same root canonicalized, which is the namespace `changed` is
    /// expressed in because that is the only namespace FSEvents answers
    /// in. Equal to `root` for every root that was already canonical.
    canonical_root: PathBuf,
    /// Every path the replay implicated (each reported path plus its
    /// parent), unfiltered, in the canonical namespace.
    changed: Vec<PathBuf>,
    /// The observation time the window replays *from*. Stored rows older
    /// than this were written before the window opened, so the window
    /// cannot vouch for them.
    since_observed_at: u64,
}

/// What this observation's FSEvents replays can vouch for -- the
/// evidence that makes reusing a previous pass's measurement honest
/// rather than hopeful.
///
/// # Why this exists
///
/// Both unit families cache a previous pass's answer: the external
/// family its folded byte totals
/// (`crate::folded_measurement::reuse_folded_measurement`), the agent
/// family a whole container's identified units
/// (`crate::agents::IdentifyCtx::container`). Until 2026-09-22 both
/// decided "unchanged" from the recorded directories' own
/// `mtime`/`ctime` stamps. A directory stamp cannot see a file rewritten
/// **in place**, and for agent storage that is not a corner case: a tool
/// appends to an open session transcript in place, which moves the
/// file's own size and mtime and not its parent's. A growth tool that
/// cannot see the file that is growing is not doing its job, so
/// stamp-only reuse was removed as a sufficient condition.
///
/// Trusted event coverage replaces it, and it is the same rule the Cargo
/// adapter has always followed (`crate::consumers::cargo`): when this
/// pass has a successful FSEvents replay (or live window) over a root,
/// and that window reports no event at or under a path, then nothing
/// under that path changed since the window opened -- appends included,
/// because FSEvents reports writes, not just directory-shape changes.
/// When there is no such window (a full walk, a refusal, an overflow, a
/// first observation), there is no evidence and there is no reuse: the
/// caller re-identifies, where the per-file caches still keep header
/// reads at zero for the files that did not move.
///
/// # What it deliberately does not do
///
/// It does not fall back to directory stamps when a window is missing. A
/// stamp is strictly weaker evidence than the window -- it cannot see
/// the append -- and costs one `stat` per recorded directory per pass,
/// so keeping it as a second opinion would buy nothing and charge for
/// it.
#[derive(Debug, Clone, Default)]
pub struct EventCoverage {
    windows: Vec<EventWindow>,
}

impl EventCoverage {
    /// No evidence at all: every reuse decision must re-derive. This is
    /// what a store-less caller, a forced full walk, an execution-time
    /// recheck and every refusal reason get.
    pub fn untrusted() -> Self {
        Self::default()
    }

    /// Records one root whose replay this pass trusted.
    ///
    /// `changed` must be the replay's own list (each reported path and
    /// its parent), *not* one filtered for exclusions or pruning: a path
    /// this walk chose not to descend into is still a path the window
    /// has to be able to say "changed" about.
    pub fn trust(&mut self, root: PathBuf, changed: Vec<PathBuf>, since_observed_at: u64) {
        let canonical_root = root.clone();
        self.trust_alias(root, canonical_root, changed, since_observed_at);
    }

    /// [`Self::trust`] for a root whose units are addressed by a
    /// non-canonical spelling (a symlinked tool home, a `/var` alias of
    /// `/private/var`).
    ///
    /// Both spellings are needed and neither substitutes for the other.
    /// Matching a queried path against the canonical root alone would
    /// never cover a unit reached through the alias; matching the
    /// alias-form path against `changed` (which FSEvents always answers
    /// in canonical form) would find no event under it and report the
    /// unit quiet *because* the spellings differ -- silently wrong in
    /// the direction that matters. So the query is matched against
    /// `root` and then rewritten into the canonical namespace before it
    /// is tested against `changed`.
    pub fn trust_alias(
        &mut self,
        root: PathBuf,
        canonical_root: PathBuf,
        changed: Vec<PathBuf>,
        since_observed_at: u64,
    ) {
        self.windows.push(EventWindow {
            root,
            canonical_root,
            changed,
            since_observed_at,
        });
    }

    /// Folds another pass-scoped coverage into this one. Windows are
    /// independent evidence, so this is a concatenation: the unit-root
    /// cursors' windows and the walk's windows both vouch for whatever
    /// they each cover.
    pub fn merge(&mut self, other: EventCoverage) {
        self.windows.extend(other.windows);
    }

    /// A single-window coverage, for tests and for callers that replay
    /// exactly one root.
    pub fn trusted(root: PathBuf, changed: Vec<PathBuf>, since_observed_at: u64) -> Self {
        let mut c = Self::untrusted();
        c.trust(root, changed, since_observed_at);
        c
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// Whether this pass can show that **nothing under `path` changed**
    /// since `stored_at`, the observation time of the rows a caller
    /// wants to reuse.
    ///
    /// Three things must all hold, and each one is a way reuse goes
    /// wrong when it is missing:
    ///
    /// * some trusted window's root is `path` or an ancestor of it --
    ///   otherwise nothing was watching this path at all;
    /// * the rows are no older than that window's start
    ///   (`stored_at >= since_observed_at`) -- otherwise a change in the
    ///   gap between when the rows were written and when the window
    ///   opened is invisible to both. That gap is exactly what a pass
    ///   which skipped this unit family (`report::ObservationParts`)
    ///   leaves behind, and it is why a window on its own is not enough;
    /// * that window reports no event at `path` or under it. An event
    ///   *above* `path` (a sibling created next door) is not a change to
    ///   `path`, and is not treated as one.
    pub fn unchanged_since(&self, path: &Path, stored_at: u64) -> bool {
        self.windows.iter().any(|w| {
            if stored_at < w.since_observed_at {
                return false;
            }
            let Ok(rel) = path.strip_prefix(&w.root) else {
                return false;
            };
            let in_window = w.canonical_root.join(rel);
            !w.changed.iter().any(|c| c.starts_with(&in_window))
        })
    }
}

/// Splits one shared stream's change list into one list per root: each
/// root gets exactly the reported paths at or under it.
///
/// Several roots on one device are replayed through a single FSEvents
/// stream (see [`FsEventsSource::replay_roots`]), which means one
/// callback sees every root's events. Handing that combined list to
/// every root would make a write under `~/.cargo` read as a change under
/// `~/.claude` -- the cross-talk this function exists to prevent. Roots
/// are compared as canonical paths, the namespace FSEvents reports in.
fn partition_changes(roots: &[PathBuf], changes: &[PathBuf]) -> Vec<Vec<PathBuf>> {
    roots
        .iter()
        .map(|root| {
            changes
                .iter()
                .filter(|c| c.starts_with(root))
                .cloned()
                .collect()
        })
        .collect()
}

/// Returns the platform's real source on macOS, and the always-refusing
/// stub everywhere else.
pub fn platform_source() -> Box<dyn FsEventsSource> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacOsFsEventsSource)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(UnsupportedPlatformSource)
    }
}

/// One delivery from a live [`watch`]: the directories FSEvents reported
/// (each with its parent, within the root) and the newest event id seen.
#[derive(Debug, Clone)]
pub struct WatchBatch {
    pub changed_dirs: Vec<PathBuf>,
    pub last_event_id: u64,
}

/// A running live stream. Dropping it, or calling `stop`, ends the
/// thread and releases the stream.
pub struct Watcher {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// A stream whose thread has been spawned but whose
/// `FSEventStreamStart` has not been confirmed yet.
///
/// The split exists because starting an FSEvents stream is a synchronous
/// round-trip to `fseventsd` that is **serialized per process** and
/// measurably costs seconds: on the development machine two streams over
/// two fresh temp directories reported ready at 1.4 s / 2.9 s on a quiet
/// run and at 4.7 s / 6.6 s on a loaded one. A caller that opens one
/// stream per root and waits for each one before spawning the next pays
/// the sum of those; with a fixed per-call budget the later roots are
/// the ones that lose, and the stream they abandon had usually started
/// successfully a moment later. Spawning every root first and only then
/// collecting readiness bounds the wait by the slowest stream instead of
/// their sum.
pub struct PendingWatch {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    ready: std::sync::mpsc::Receiver<bool>,
}

impl PendingWatch {
    /// Waits up to `budget` for this stream to report that
    /// `FSEventStreamStart` succeeded. A stream that reports failure, or
    /// that has not reported at all within the budget, is stopped and
    /// joined: an abandoned stream must not outlive the decision to
    /// abandon it.
    pub fn ready(mut self, budget: std::time::Duration) -> Option<Watcher> {
        match self.ready.recv_timeout(budget) {
            Ok(true) => Some(Watcher {
                stop: self.stop.clone(),
                thread: self.thread.take(),
            }),
            _ => {
                self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
                if let Some(t) = self.thread.take() {
                    let _ = t.join();
                }
                None
            }
        }
    }
}

impl Drop for PendingWatch {
    fn drop(&mut self) {
        // Only a stream still owned by this pending handle is stopped.
        // `ready` hands the join handle to the `Watcher` it returns and
        // leaves `thread` empty; setting the shared stop flag here
        // regardless would end the stream the caller just took
        // ownership of, on the same line it took it.
        if let Some(t) = self.thread.take() {
            self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = t.join();
        }
    }
}

/// How long a caller waits for one stream to confirm it started.
///
/// Was five seconds, which is smaller than the observed cost of the
/// *second* `FSEventStreamStart` in a process and is exactly why
/// `tui::app::start_watch` intermittently ended up with one watcher for
/// two roots. The budget's job is to stop a caller hanging forever on an
/// `fseventsd` that never answers, not to second-guess how long a call
/// that is known to take seconds is allowed to take. Overridable via
/// `SWAMP_FSEVENTS_WATCH_START_TIMEOUT_SEC`.
pub fn watch_start_budget() -> std::time::Duration {
    std::env::var("SWAMP_FSEVENTS_WATCH_START_TIMEOUT_SEC")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(std::time::Duration::from_secs)
        .unwrap_or(std::time::Duration::from_secs(30))
}

impl Watcher {
    pub fn stop(mut self) {
        self.signal_stop();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
    fn signal_stop(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.signal_stop();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Starts a live FSEvents stream on `root` from now, delivering a
/// [`WatchBatch`] on `tx` each time FSEvents flushes (latency 0.5 s). The
/// stream runs on its own thread with its own run loop. `None` where the
/// platform has no FSEvents.
///
/// Blocks until the stream confirms it started (see
/// [`watch_start_budget`]). A caller opening several streams should use
/// [`watch_pending`] and collect readiness afterwards instead, so the
/// per-stream `fseventsd` latencies overlap rather than add up.
pub fn watch(root: &Path, tx: std::sync::mpsc::Sender<WatchBatch>) -> Option<Watcher> {
    watch_pending(root, tx)?.ready(watch_start_budget())
}

/// Spawns `root`'s stream thread and returns immediately; the caller
/// decides when (and for how long) to wait for it to report ready.
/// `None` where the platform has no FSEvents, or where the thread itself
/// could not be spawned.
pub fn watch_pending(root: &Path, tx: std::sync::mpsc::Sender<WatchBatch>) -> Option<PendingWatch> {
    #[cfg(target_os = "macos")]
    {
        macos::watch_pending(root, tx)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (root, tx);
        None
    }
}

/// The seam `tui::app::start_watch` opens its streams through, so a test
/// can assert "one stream per root" without depending on `fseventsd`
/// answering within any particular time.
pub type WatchFactory = fn(&Path, std::sync::mpsc::Sender<WatchBatch>) -> Option<PendingWatch>;

/// The non-macOS fallback: always refuses, naming the platform as the
/// cause, never a bug in the replay itself.
pub struct UnsupportedPlatformSource;

impl FsEventsSource for UnsupportedPlatformSource {
    fn replay(&self, _request: &FsEventsRequest) -> FsEventsPlan {
        FsEventsPlan::refuse(RefreshRefusal::UnsupportedPlatform, 0, None)
    }
}

/// Adds `path` and its parent directory to `changes`, provided each is
/// within `root`. FSEvents reports a path whose meaning (the changed item
/// itself, or the directory containing it) depends on flags this design
/// deliberately does not trust for that judgement (see
/// `docs/fsevents-helper.md`'s "every path contributes itself and its
/// parent"); adding both costs one extra listing on re-walk and closes
/// both a creation and a deletion.
fn add_with_parent(changes: &mut std::collections::HashSet<PathBuf>, root: &Path, path: &Path) {
    for candidate in [
        Some(path.to_path_buf()),
        path.parent().map(Path::to_path_buf),
    ]
    .into_iter()
    .flatten()
    {
        if candidate == root || candidate.starts_with(root) {
            changes.insert(candidate);
        }
    }
}

/// Legacy pre-#29 shape kept only so the stub's original two symbols
/// (`full_refresh`, an early `RefreshRefusal` with fewer variants) do not
/// silently disappear from anyone who imported them mid-restart. Superseded
/// by [`FsEventsPlan::refuse`] internally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshResult {
    pub incremental: bool,
    pub refusal: Option<RefreshRefusal>,
    pub meta: FactMeta,
}

pub fn full_refresh(reason: RefreshRefusal) -> RefreshResult {
    RefreshResult {
        incremental: false,
        refusal: Some(reason),
        meta: FactMeta::now("filesystem.full-refresh", Confidence::High),
    }
}

#[cfg(target_os = "macos")]
#[allow(deprecated)] // fsevent-sys itself is marked deprecated in favour of
// objc2-core-services, but it is a maintained, direct binding to the same
// stable CoreServices C API this module needs and ships today; swapping
// bindings crates is out of scope for this change.
mod macos {
    use super::*;
    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::TCFType;
    use core_foundation::runloop::{CFRunLoop, kCFRunLoopDefaultMode};
    use core_foundation::string::CFString;
    use fsevent_sys as fs;
    use std::ffi::CStr;
    use std::os::raw::c_void;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::time::{Duration, Instant};

    /// Bounds one replay's run loop. FSEvents replays retained history
    /// from local, in-kernel state, so a replay that has not reached
    /// `HistoryDone` in this long is not going to; refusing with
    /// `HelperInconclusive` beats blocking an observation indefinitely.
    /// Overridable for slow CI via `SWAMP_FSEVENTS_TIMEOUT_SEC`.
    fn replay_budget() -> Duration {
        std::env::var("SWAMP_FSEVENTS_TIMEOUT_SEC")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(6))
    }

    struct Collector {
        /// Every root this stream is watching. One stream can carry
        /// several roots on the same device; the collector keeps their
        /// union and `replay_many` splits it per root afterwards.
        roots: Vec<PathBuf>,
        changes: std::collections::HashSet<PathBuf>,
        history_done: bool,
        hard_fail: Option<RefreshRefusal>,
    }

    extern "C" fn stream_callback(
        _stream: fs::FSEventStreamRef,
        info: *mut c_void,
        num_events: usize,
        event_paths: *mut c_void,
        event_flags: *const fs::FSEventStreamEventFlags,
        _event_ids: *const fs::FSEventStreamEventId,
    ) {
        if info.is_null() || num_events == 0 {
            return;
        }
        // SAFETY: `info` is the `Collector` this replay's caller created
        // and kept alive on the stack for the stream's entire lifetime;
        // FSEvents only invokes this callback on the run loop that same
        // stack frame is pumping.
        let collector = unsafe { &mut *(info as *mut Collector) };
        let paths = event_paths as *const *const std::os::raw::c_char;
        // SAFETY: FSEvents guarantees `num_events` valid entries in both
        // the flags array and the paths array it hands the callback.
        let flags = unsafe { std::slice::from_raw_parts(event_flags, num_events) };

        for (i, &f) in flags.iter().enumerate() {
            if (f & fs::kFSEventStreamEventFlagMustScanSubDirs) != 0
                || (f & fs::kFSEventStreamEventFlagEventIdsWrapped) != 0
            {
                collector
                    .hard_fail
                    .get_or_insert(RefreshRefusal::HelperInconclusive);
                continue;
            }
            if (f & fs::kFSEventStreamEventFlagRootChanged) != 0
                || (f & fs::kFSEventStreamEventFlagUnmount) != 0
            {
                collector
                    .hard_fail
                    .get_or_insert(RefreshRefusal::RootMismatch);
                continue;
            }
            if (f & fs::kFSEventStreamEventFlagHistoryDone) != 0 {
                collector.history_done = true;
                CFRunLoop::get_current().stop();
                continue;
            }

            // SAFETY: `paths` was validated non-null by FSEvents for
            // every one of `num_events` entries; `i` is in range.
            let cpath = unsafe { *paths.add(i) };
            if cpath.is_null() {
                // A path we cannot read is a change we cannot locate:
                // refusing is the difference between a full refresh and a
                // generation that silently omits this event.
                collector
                    .hard_fail
                    .get_or_insert(RefreshRefusal::HelperInconclusive);
                continue;
            }
            // SAFETY: FSEvents paths are NUL-terminated C strings.
            let cstr = unsafe { CStr::from_ptr(cpath) };
            let path = PathBuf::from(std::ffi::OsStr::from_bytes(cstr.to_bytes()));
            for i in 0..collector.roots.len() {
                let root = collector.roots[i].clone();
                add_with_parent(&mut collector.changes, &root, &path);
            }
        }
    }

    /// State behind the live stream's callback: where batches go.
    struct WatchState {
        root: PathBuf,
        tx: std::sync::mpsc::Sender<super::WatchBatch>,
    }

    extern "C" fn watch_callback(
        _stream: fs::FSEventStreamRef,
        info: *mut c_void,
        num_events: usize,
        event_paths: *mut c_void,
        event_flags: *const fs::FSEventStreamEventFlags,
        event_ids: *const fs::FSEventStreamEventId,
    ) {
        if info.is_null() || num_events == 0 {
            return;
        }
        // SAFETY: `info` is the `WatchState` the watch thread boxed and
        // keeps alive until after the stream is invalidated; FSEvents
        // invokes this callback only on that thread's run loop.
        let state = unsafe { &*(info as *const WatchState) };
        let paths = event_paths as *const *const std::os::raw::c_char;
        // SAFETY: FSEvents guarantees `num_events` valid entries in the
        // flags, paths and ids arrays.
        let flags = unsafe { std::slice::from_raw_parts(event_flags, num_events) };
        let ids = unsafe { std::slice::from_raw_parts(event_ids, num_events) };
        let mut changes = std::collections::HashSet::new();
        let mut last_id = 0u64;
        for (i, &f) in flags.iter().enumerate() {
            last_id = last_id.max(ids[i]);
            if (f & fs::kFSEventStreamEventFlagMustScanSubDirs) != 0
                || (f & fs::kFSEventStreamEventFlagRootChanged) != 0
                || (f & fs::kFSEventStreamEventFlagUnmount) != 0
            {
                // Everything under the root may have changed: report the
                // root itself; the consumer decides between an incremental
                // re-walk of it and a full walk.
                changes.insert(state.root.clone());
                continue;
            }
            // SAFETY: validated non-null by FSEvents; `i` in range.
            let cpath = unsafe { *paths.add(i) };
            if cpath.is_null() {
                continue;
            }
            // SAFETY: FSEvents paths are NUL-terminated C strings.
            let cstr = unsafe { CStr::from_ptr(cpath) };
            let path = PathBuf::from(std::ffi::OsStr::from_bytes(cstr.to_bytes()));
            add_with_parent(&mut changes, &state.root, &path);
        }
        if !changes.is_empty() {
            let _ = state.tx.send(super::WatchBatch {
                changed_dirs: changes.into_iter().collect(),
                last_event_id: last_id,
            });
        }
    }

    pub fn watch_pending(
        root: &Path,
        tx: std::sync::mpsc::Sender<super::WatchBatch>,
    ) -> Option<super::PendingWatch> {
        use std::sync::atomic::{AtomicBool, Ordering};
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<bool>();
        let thread = std::thread::Builder::new()
            .name("fsevents-watch".into())
            .spawn(move || {
                let state = Box::new(WatchState {
                    root: root.clone(),
                    tx,
                });
                let info_ptr = state.as_ref() as *const WatchState as *mut c_void;
                let cf_path = CFString::new(&root.to_string_lossy());
                let paths_array: CFArray<CFString> = CFArray::from_CFTypes(&[cf_path]);
                let context = fs::FSEventStreamContext {
                    version: 0,
                    info: info_ptr,
                    retain: None,
                    release: None,
                    copy_description: None,
                };
                let create_flags =
                    fs::kFSEventStreamCreateFlagNoDefer | fs::kFSEventStreamCreateFlagWatchRoot;
                // SAFETY: `paths_array`, `context` and `state` outlive the
                // stream, which is released below before they drop.
                let stream = unsafe {
                    fs::FSEventStreamCreate(
                        core_foundation_sys::base::kCFAllocatorDefault,
                        watch_callback,
                        &context,
                        paths_array.as_concrete_TypeRef() as CFArrayRef,
                        fs::kFSEventStreamEventIdSinceNow,
                        0.5,
                        create_flags,
                    )
                };
                if stream.is_null() {
                    let _ = ready_tx.send(false);
                    return;
                }
                // SAFETY: stream just created; released on every exit below.
                let started = unsafe {
                    fs::FSEventStreamScheduleWithRunLoop(
                        stream,
                        CFRunLoop::get_current().as_concrete_TypeRef(),
                        kCFRunLoopDefaultMode,
                    );
                    fs::FSEventStreamStart(stream) != 0
                };
                let _ = ready_tx.send(started);
                if started {
                    while !stop_thread.load(Ordering::Relaxed) {
                        CFRunLoop::run_in_mode(
                            unsafe { kCFRunLoopDefaultMode },
                            Duration::from_millis(250),
                            true,
                        );
                    }
                    // SAFETY: matches the successful start above.
                    unsafe {
                        fs::FSEventStreamStop(stream);
                    }
                }
                // SAFETY: matches `FSEventStreamCreate` above.
                unsafe {
                    fs::FSEventStreamInvalidate(stream);
                    fs::FSEventStreamRelease(stream);
                }
                drop(state);
            })
            .ok()?;
        Some(super::PendingWatch {
            stop,
            thread: Some(thread),
            ready: ready_rx,
        })
    }

    pub struct MacOsFsEventsSource;

    impl FsEventsSource for MacOsFsEventsSource {
        fn replay(&self, request: &FsEventsRequest) -> FsEventsPlan {
            self.replay_roots(std::slice::from_ref(request))
                .pop()
                .unwrap_or_else(|| {
                    FsEventsPlan::refuse(RefreshRefusal::FseventsdUnavailable, 0, None)
                })
        }

        fn replay_roots(&self, requests: &[FsEventsRequest]) -> Vec<FsEventsPlan> {
            replay_many(requests)
        }
    }

    /// Every root whose pre-checks passed, grouped by the device its
    /// FSEvents history lives on.
    struct Group {
        dev: u64,
        /// Index into the caller's request slice, so each plan goes back
        /// to the root that asked for it.
        members: Vec<usize>,
        /// The earliest anchor among the members. A member whose own
        /// anchor is later simply sees some events it already knew
        /// about, which can only make it re-measure something that did
        /// not need it -- never the other way round.
        since_id: u64,
    }

    fn replay_many(requests: &[FsEventsRequest]) -> Vec<FsEventsPlan> {
        // SAFETY: no arguments; a pure query of the FSEvents subsystem.
        let current = unsafe { fs::FSEventsGetCurrentEventId() };
        if current == 0 {
            return requests
                .iter()
                .map(|_| FsEventsPlan::refuse(RefreshRefusal::FseventsdUnavailable, 0, None))
                .collect();
        }

        let mut plans: Vec<Option<FsEventsPlan>> = vec![None; requests.len()];
        let mut groups: Vec<Group> = Vec::new();

        for (i, request) in requests.iter().enumerate() {
            let root = &request.root;
            let since = &request.since;
            let dev = match std::fs::metadata(root) {
                Ok(meta) => meta.dev(),
                Err(_) => {
                    plans[i] = Some(FsEventsPlan::refuse(
                        RefreshRefusal::FseventsdUnavailable,
                        current,
                        None,
                    ));
                    continue;
                }
            };

            // Sanity check named in the issue: ask FSEvents for the last
            // event id it can vouch for on this device as of "now". This
            // is read-only and its only use here is to catch a device
            // whose FSEvents history log cannot possibly reach back to
            // `since` (the id looks plausible but predates everything
            // retained).
            // SAFETY: `dev` came from a real `stat`;
            // `CFAbsoluteTimeGetCurrent` takes no arguments.
            let last_known = unsafe {
                fs::FSEventsGetLastEventIdForDeviceBeforeTime(
                    dev,
                    core_foundation_sys::date::CFAbsoluteTimeGetCurrent(),
                )
            };

            if let Some(stored_device) = since.device
                && stored_device != dev
            {
                plans[i] = Some(FsEventsPlan::refuse(
                    RefreshRefusal::RootMismatch,
                    current,
                    Some(dev),
                ));
                continue;
            }
            let Some(since_id) = since.event_id else {
                plans[i] = Some(FsEventsPlan::refuse(
                    RefreshRefusal::NoStoredEventId,
                    current,
                    Some(dev),
                ));
                continue;
            };
            if since_id > current {
                plans[i] = Some(FsEventsPlan::refuse(
                    RefreshRefusal::EventIdFromFuture,
                    current,
                    Some(dev),
                ));
                continue;
            }
            // `last_known` being 0 means FSEvents could not answer at all
            // for this device (no history yet observed); that is not by
            // itself a reason to refuse a replay FSEvents is about to
            // attempt, so it only gates the case where FSEvents can
            // positively vouch for a *later* floor than our stored id,
            // meaning `since_id` is stale history that has already
            // rotated out.
            if last_known != 0 && since_id != 0 && since_id < last_known {
                plans[i] = Some(FsEventsPlan::refuse(
                    RefreshRefusal::HelperInconclusive,
                    current,
                    Some(dev),
                ));
                continue;
            }

            match groups.iter_mut().find(|g| g.dev == dev) {
                Some(g) => {
                    g.members.push(i);
                    g.since_id = g.since_id.min(since_id);
                }
                None => groups.push(Group {
                    dev,
                    members: vec![i],
                    since_id,
                }),
            }
        }

        for group in groups {
            let roots: Vec<PathBuf> = group
                .members
                .iter()
                .map(|&i| requests[i].root.clone())
                .collect();
            match run_stream(&roots, group.since_id) {
                Err(reason) => {
                    for &i in &group.members {
                        plans[i] = Some(FsEventsPlan::refuse(reason, current, Some(group.dev)));
                    }
                }
                Ok(changes) => {
                    let per_root = super::partition_changes(&roots, &changes);
                    for (&i, changed) in group.members.iter().zip(per_root) {
                        plans[i] = Some(FsEventsPlan::ok(changed, current, Some(group.dev)));
                    }
                }
            }
        }

        plans
            .into_iter()
            .map(|p| {
                p.unwrap_or_else(|| {
                    FsEventsPlan::refuse(RefreshRefusal::FseventsdUnavailable, current, None)
                })
            })
            .collect()
    }

    /// One FSEvents stream over `roots` (all on one device), replayed
    /// from `since_id`. `Ok` is the complete union of implicated paths;
    /// `Err` is the one refusal every root in the group shares, because
    /// an inconclusive replay is inconclusive for all of them.
    fn run_stream(roots: &[PathBuf], since_id: u64) -> Result<Vec<PathBuf>, RefreshRefusal> {
        let mut collector = Box::new(Collector {
            roots: roots.to_vec(),
            changes: std::collections::HashSet::new(),
            history_done: false,
            hard_fail: None,
        });
        let info_ptr = collector.as_mut() as *mut Collector as *mut c_void;

        let cf_paths: Vec<CFString> = roots
            .iter()
            .map(|r| CFString::new(&r.to_string_lossy()))
            .collect();
        let paths_array: CFArray<CFString> = CFArray::from_CFTypes(&cf_paths);
        let context = fs::FSEventStreamContext {
            version: 0,
            info: info_ptr,
            retain: None,
            release: None,
            copy_description: None,
        };
        // NoDefer delivers the first batch immediately instead of after
        // the latency window; WatchRoot reports the root itself being
        // moved or replaced. Directory-level events (no FileEvents flag)
        // suffice: rows here are per artifact/worktree/dir, never
        // per-file.
        let create_flags =
            fs::kFSEventStreamCreateFlagNoDefer | fs::kFSEventStreamCreateFlagWatchRoot;

        // SAFETY: `paths_array` and `context` outlive the call; the
        // callback pointer has the exact signature FSEvents expects.
        let stream = unsafe {
            fs::FSEventStreamCreate(
                core_foundation_sys::base::kCFAllocatorDefault,
                stream_callback,
                &context,
                paths_array.as_concrete_TypeRef() as CFArrayRef,
                since_id,
                0.0,
                create_flags,
            )
        };
        if stream.is_null() {
            return Err(RefreshRefusal::FseventsdUnavailable);
        }

        // SAFETY: `stream` was just created and is released below on
        // every path out of this function.
        unsafe {
            fs::FSEventStreamScheduleWithRunLoop(
                stream,
                CFRunLoop::get_current().as_concrete_TypeRef(),
                kCFRunLoopDefaultMode,
            );
            if fs::FSEventStreamStart(stream) == 0 {
                fs::FSEventStreamInvalidate(stream);
                fs::FSEventStreamRelease(stream);
                return Err(RefreshRefusal::FseventsdUnavailable);
            }
        }

        let deadline = Instant::now() + replay_budget();
        while !collector.history_done && collector.hard_fail.is_none() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            // Run in short slices so the loop condition (history_done /
            // hard_fail, both set from inside the callback which runs on
            // this same thread while this call is blocked in it) is
            // re-checked promptly rather than only after the whole
            // remaining budget elapses.
            let slice = remaining.min(Duration::from_millis(200));
            CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, slice, true);
        }

        // SAFETY: matches the successful `FSEventStreamCreate` above.
        unsafe {
            fs::FSEventStreamStop(stream);
            fs::FSEventStreamInvalidate(stream);
            fs::FSEventStreamRelease(stream);
        }

        if let Some(reason) = collector.hard_fail {
            return Err(reason);
        }
        if !collector.history_done {
            // The run loop stopped (budget exhausted) without FSEvents
            // ever reporting the replay complete. Whatever was collected
            // may be a prefix of what changed, and a prefix is exactly
            // the silent partial this design refuses to report.
            return Err(RefreshRefusal::HelperInconclusive);
        }

        Ok(collector.changes.drain().collect())
    }
}

/// Test-only support kept as an ordinary (non-`#[cfg(test)]`) module so
/// integration tests in `crates/core/tests/*.rs` -- which link against
/// this crate rather than compiling into it -- can use it too. Small and
/// inert in a release binary: one struct, one trait impl, no I/O.
pub mod testing {
    use super::{FsEventsPlan, FsEventsRequest, FsEventsSource, PendingWatch, WatchBatch};
    use std::path::Path;

    /// A canned source: returns whatever plan it was built with,
    /// regardless of the request. Lets every refusal reason and the
    /// incremental path be exercised without depending on the live
    /// `fseventsd`.
    pub struct CannedSource(pub FsEventsPlan);

    impl FsEventsSource for CannedSource {
        fn replay(&self, _request: &FsEventsRequest) -> FsEventsPlan {
            self.0.clone()
        }
    }

    /// A [`super::WatchFactory`] that opens no FSEvents stream and
    /// reports ready immediately.
    ///
    /// It exists so "one stream per root" can be asserted as a property
    /// of the caller's loop rather than of how quickly `fseventsd`
    /// answers. Starting a real stream is a multi-second, per-process
    /// serialized call (see [`super::PendingWatch`]), which makes any
    /// test that opens two real streams a race against its own budget.
    pub fn inert_watch_factory(
        _root: &Path,
        _tx: std::sync::mpsc::Sender<WatchBatch>,
    ) -> Option<PendingWatch> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let _ = ready_tx.send(true);
        Some(PendingWatch {
            stop: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            thread: None,
            ready: ready_rx,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[cfg(target_os = "macos")]
    #[test]
    fn live_watch_reports_a_write_under_the_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let watcher = watch(&root, tx).expect("fseventsd available on macOS");
        std::thread::sleep(Duration::from_millis(300));
        let dir = root.join("proj");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();
        let mut seen = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        while std::time::Instant::now() < deadline {
            if let Ok(b) = rx.recv_timeout(Duration::from_millis(200)) {
                seen.extend(b.changed_dirs);
                if seen.iter().any(|p| p == &dir) {
                    break;
                }
            }
        }
        watcher.stop();
        assert!(
            seen.iter().any(|p| p == &dir),
            "expected {} among live changes, got {seen:?}",
            dir.display()
        );
    }

    #[test]
    fn unsupported_platform_source_always_refuses() {
        let source = UnsupportedPlatformSource;
        let plan = source.replay(&FsEventsRequest {
            root: PathBuf::from("/tmp"),
            since: FsEventsState {
                event_id: Some(1),
                device: Some(1),
                last_observed_at: Some(1),
                rules_version: 0,
                unit_root: None,
            },
        });
        assert!(!plan.incremental);
        assert_eq!(plan.refusal, Some(RefreshRefusal::UnsupportedPlatform));
        assert_eq!(plan.reason_str(), "unsupported_platform");
    }

    #[test]
    fn add_with_parent_stays_within_root() {
        let root = Path::new("/root");
        let mut changes = std::collections::HashSet::new();
        add_with_parent(&mut changes, root, Path::new("/root/a/b"));
        assert!(changes.contains(Path::new("/root/a/b")));
        assert!(changes.contains(Path::new("/root/a")));

        let mut changes = std::collections::HashSet::new();
        add_with_parent(&mut changes, root, Path::new("/elsewhere/x"));
        assert!(changes.is_empty(), "a path outside root must not be added");
    }

    /// Two roots on one device share one FSEvents stream, so the
    /// callback sees both roots' events. Splitting that union is the
    /// only thing standing between "a write under the Cargo home" and
    /// "the Claude Code home changed"; assert the split directly, since
    /// the stream itself cannot be reproduced deterministically.
    #[test]
    fn a_shared_stream_gives_each_root_only_its_own_changes() {
        let cargo = PathBuf::from("/Users/x/.cargo");
        let claude = PathBuf::from("/Users/x/.claude");
        let changes = vec![
            cargo.join("registry/cache"),
            cargo.join("registry"),
            PathBuf::from("/Users/x/elsewhere"),
        ];
        let split = partition_changes(&[cargo.clone(), claude.clone()], &changes);
        assert_eq!(split[0].len(), 2, "the Cargo home keeps its own two paths");
        assert!(
            split[1].is_empty(),
            "the Claude Code home saw nothing: {:?}",
            split[1]
        );
        // And the consequence the split exists for.
        let mut coverage = EventCoverage::untrusted();
        coverage.trust(cargo.clone(), split[0].clone(), 1_000);
        coverage.trust(claude.clone(), split[1].clone(), 1_000);
        assert!(
            !coverage.unchanged_since(&cargo, 1_000),
            "the root that changed must not be reported unchanged"
        );
        assert!(
            coverage.unchanged_since(&claude, 1_000),
            "the quiet root on the same stream is still reusable"
        );
    }

    /// A window whose root is reached through an alias spelling
    /// (`/var/folders/...` for `/private/var/folders/...`) must translate
    /// the queried path into the canonical namespace before testing it
    /// against the change list. Skipping the translation reports the unit
    /// quiet because the spellings differ, which is the wrong answer in
    /// the only direction that matters.
    #[test]
    fn an_aliased_root_still_sees_its_own_changes() {
        let alias = PathBuf::from("/var/home/.claude");
        let canonical = PathBuf::from("/private/var/home/.claude");
        let mut coverage = EventCoverage::untrusted();
        coverage.trust_alias(
            alias.clone(),
            canonical.clone(),
            vec![canonical.join("projects/a")],
            1_000,
        );
        assert!(
            !coverage.unchanged_since(&alias.join("projects/a"), 1_000),
            "the change is under the queried path, however it is spelled"
        );
        assert!(
            coverage.unchanged_since(&alias.join("projects/b"), 1_000),
            "a sibling that did not change is still reusable"
        );
        assert!(
            !coverage.unchanged_since(&PathBuf::from("/somewhere/else"), 1_000),
            "a path outside the root is not covered at all"
        );
    }

    /// Merging is concatenation: the unit-root cursors' windows and the
    /// walk's windows are independent evidence and neither overrides the
    /// other.
    #[test]
    fn merging_coverage_keeps_both_sets_of_windows() {
        let mut walk = EventCoverage::trusted(PathBuf::from("/src"), Vec::new(), 1_000);
        let units = EventCoverage::trusted(PathBuf::from("/home/.cargo"), Vec::new(), 1_000);
        walk.merge(units);
        assert!(walk.unchanged_since(Path::new("/src/proj"), 1_000));
        assert!(walk.unchanged_since(Path::new("/home/.cargo/registry"), 1_000));
        assert!(!walk.unchanged_since(Path::new("/home/.claude"), 1_000));
    }

    #[test]
    fn reason_str_covers_every_refusal() {
        for (reason, expected) in [
            (RefreshRefusal::NoStoredEventId, "no_stored_event_id"),
            (RefreshRefusal::EventIdFromFuture, "event_id_from_future"),
            (RefreshRefusal::RootMismatch, "root_mismatch"),
            (
                RefreshRefusal::FseventsdUnavailable,
                "fseventsd_unavailable",
            ),
            (RefreshRefusal::HelperInconclusive, "helper_inconclusive"),
            (RefreshRefusal::TooManyChanges, "too_many_changes"),
            (RefreshRefusal::TooSoon, "too_soon"),
            (RefreshRefusal::UnsupportedPlatform, "unsupported_platform"),
        ] {
            assert_eq!(reason.as_str(), expected);
        }
    }
}
