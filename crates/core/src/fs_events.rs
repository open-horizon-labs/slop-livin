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
    /// This platform's kernel keeps no persisted change history to
    /// replay from. Linux: inotify reports only what happens while a
    /// watch is open, so a period with no running watch is a gap, not a
    /// quiet period. Distinct from [`RefreshRefusal::UnsupportedPlatform`]
    /// (which says a backend is missing) because this one says the
    /// backend cannot exist: no amount of implementation work makes a
    /// Linux kernel able to answer "what changed while you were not
    /// running". #81 adds the live watch, which narrows the gap to the
    /// time before the watch opened; it does not remove it.
    NoPersistedChangeHistory,
    /// Linux live watch: the kernel's inotify queue overflowed
    /// (`IN_Q_OVERFLOW`) and dropped events. Nothing since the watch's
    /// epoch opened can be vouched for.
    WatchQueueOverflow,
    /// Linux live watch: `inotify_add_watch` ran out of watches
    /// (`fs.inotify.max_user_watches`), so part of the root is unwatched.
    WatchLimitReached,
    /// Linux live watch: a directory under the root could not be watched
    /// or listed.
    WatchPermissionGap,
    /// Linux live watch: a watched filesystem was unmounted.
    WatchedFilesystemUnmounted,
    /// Linux live watch: the kernel removed a watch swamp did not ask it
    /// to remove.
    WatchRemoved,
    /// Linux: a live watch (TUI or `swamp collect`) is running, but the
    /// stored observation of this root predates its epoch -- the gap
    /// between the two is not covered by anything.
    LiveWatchGap,
    /// Linux: a collector checkpoint exists for this root, but its
    /// collector is not running (it exited, was killed, or the machine
    /// rebooted), so the checkpoint says nothing about what happened
    /// since.
    CollectorStopped,
    /// Linux: the collector did not confirm it had drained its event
    /// queue in time, so its checkpoint might miss a change that has
    /// already happened.
    CollectorUnresponsive,
    /// Linux: the running collector's scope (root identity or
    /// exclusions) is not the scope this observation walks.
    ScopeChanged,
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
            RefreshRefusal::NoPersistedChangeHistory => "no_persisted_change_history",
            RefreshRefusal::WatchQueueOverflow => "watch_queue_overflow",
            RefreshRefusal::WatchLimitReached => "watch_limit_reached",
            RefreshRefusal::WatchPermissionGap => "watch_permission_gap",
            RefreshRefusal::WatchedFilesystemUnmounted => "watched_filesystem_unmounted",
            RefreshRefusal::WatchRemoved => "watch_removed",
            RefreshRefusal::LiveWatchGap => "live_watch_gap",
            RefreshRefusal::CollectorStopped => "collector_stopped",
            RefreshRefusal::CollectorUnresponsive => "collector_unresponsive",
            RefreshRefusal::ScopeChanged => "scope_changed",
        }
    }

    /// Every variant, for the tests that hold the vocabulary to the
    /// docs.
    pub const ALL: &'static [RefreshRefusal] = &[
        RefreshRefusal::NoStoredEventId,
        RefreshRefusal::EventIdFromFuture,
        RefreshRefusal::RootMismatch,
        RefreshRefusal::FseventsdUnavailable,
        RefreshRefusal::HelperInconclusive,
        RefreshRefusal::TooManyChanges,
        RefreshRefusal::TooSoon,
        RefreshRefusal::UnsupportedPlatform,
        RefreshRefusal::NoPersistedChangeHistory,
        RefreshRefusal::WatchQueueOverflow,
        RefreshRefusal::WatchLimitReached,
        RefreshRefusal::WatchPermissionGap,
        RefreshRefusal::WatchedFilesystemUnmounted,
        RefreshRefusal::WatchRemoved,
        RefreshRefusal::LiveWatchGap,
        RefreshRefusal::CollectorStopped,
        RefreshRefusal::CollectorUnresponsive,
        RefreshRefusal::ScopeChanged,
    ];

    /// The sentence a coverage note or `--json` explanation carries
    /// beside the code. A reason code tells a script what happened; this
    /// tells a person why a Linux run walks fully every time and that it
    /// is the platform, not a misconfiguration.
    pub fn explanation(self) -> Option<&'static str> {
        match self {
            RefreshRefusal::NoPersistedChangeHistory => {
                crate::platform::ContinuitySource::LiveWatchEpochOnly.no_history_reason()
            }
            RefreshRefusal::WatchQueueOverflow => Some(
                "the kernel's inotify queue overflowed and dropped events, so the watch cannot \
                 say what changed; a full walk re-establishes the baseline and a new watch epoch \
                 starts from it",
            ),
            RefreshRefusal::WatchLimitReached => Some(
                "part of the root has no inotify watch because fs.inotify.max_user_watches was \
                 reached; raise it (sysctl, as root) or narrow the scope. Until then every \
                 observation of this root walks fully",
            ),
            RefreshRefusal::WatchPermissionGap => Some(
                "a directory under the root could not be watched or listed, so changes there \
                 are invisible to the watch; every observation of this root walks fully",
            ),
            RefreshRefusal::WatchedFilesystemUnmounted => {
                Some("a watched filesystem was unmounted; the watch no longer covers it")
            }
            RefreshRefusal::WatchRemoved => Some(
                "the kernel removed a watch swamp did not ask it to remove, so part of the root \
                 went unwatched",
            ),
            RefreshRefusal::LiveWatchGap => Some(
                "a live watch is running, but the last observation of this root happened before \
                 it started; the time in between is covered by nothing, so this walk is full \
                 and the next one can use the watch",
            ),
            RefreshRefusal::CollectorStopped => Some(
                "the collector that kept this root's change list is not running (it exited, was \
                 stopped, or the machine restarted), so its list says nothing about what changed \
                 since; `swamp collect` (or `swamp schedule --collector`) starts one",
            ),
            RefreshRefusal::CollectorUnresponsive => Some(
                "the collector did not confirm it had read every pending event in time, so its \
                 change list might be missing one that already happened",
            ),
            RefreshRefusal::ScopeChanged => Some(
                "the running collector watches a different scope (root identity or exclusions) \
                 than this observation walks; restart it with the current scope",
            ),
            _ => None,
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
    /// The store this observation writes to. A live-watch source finds a
    /// collector's checkpoint there; FSEvents ignores it.
    pub swamp_dir: Option<PathBuf>,
    /// What this observation excludes under `root`. A live-watch source
    /// refuses when its own watch excludes something this walks.
    pub excluded: Vec<PathBuf>,
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
    /// Linux collector: which dirty-list entries this plan's walk covers.
    /// Applied only after the observation's history is written
    /// (`growth::ObservationCheckpoint::commit`). `None` everywhere else.
    pub consume: Option<crate::continuity::Consumption>,
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
            consume: None,
        }
    }

    /// A refusal a live consumer builds itself: the watch lost coverage,
    /// the collector stopped, or the stored observation predates the
    /// watch's epoch. The walk is full, and `reason` is what the coverage
    /// note names.
    ///
    /// Marked `live`: the replay-lag floor ([`RefreshRefusal::TooSoon`])
    /// protects a replay of a *persisted* log that may lag a write, and
    /// no persisted log was consulted here, so it must not replace the
    /// actual reason with `too_soon`.
    pub fn refused(reason: RefreshRefusal, device: Option<u64>) -> Self {
        Self {
            live: true,
            ..Self::refuse(reason, 0, device)
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
            consume: None,
        }
    }

    /// The successful counterpart of [`FsEventsPlan::refuse`], built
    /// only by a backend that actually replayed something. Target-gated
    /// because on a platform with no replay backend there is no caller
    /// and no way to reach it -- and an unreachable constructor for
    /// "incremental: true" is exactly the thing a Linux build should
    /// not contain.
    #[cfg(target_os = "macos")]
    fn ok(changed_dirs: Vec<PathBuf>, current_event_id: u64, device: Option<u64>) -> Self {
        Self {
            incremental: true,
            refusal: None,
            changed_dirs,
            current_event_id,
            device,
            live: false,
            consume: None,
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
    /// Canonical, as FSEvents answers.
    root: PathBuf,
    /// Every path the replay implicated (each reported path plus its
    /// parent), unfiltered.
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
        self.windows.push(EventWindow {
            root,
            changed,
            since_observed_at,
        });
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
            path.starts_with(&w.root)
                && stored_at >= w.since_observed_at
                && !w.changed.iter().any(|c| c.starts_with(path))
        })
    }
}

/// Returns the platform's real source on macOS, and the always-refusing
/// stub everywhere else.
pub fn platform_source() -> Box<dyn FsEventsSource> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacOsFsEventsSource)
    }
    // Linux: a running collector's checkpoint when there is one and it
    // can vouch for the gap; the platform's own refusal otherwise
    // (`crate::continuity`).
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(crate::continuity::CollectorSource)
    }
}

/// One delivery from a live [`watch`]: the directories the platform's
/// watcher reported (each with its parent, within the root) and the
/// newest event id (FSEvents) or sequence number (inotify) seen.
#[derive(Debug, Clone, Default)]
pub struct WatchBatch {
    /// The root the watch is on (the key a consumer with several roots
    /// needs for a batch that names no path: an epoch opening, a loss).
    pub root: PathBuf,
    pub changed_dirs: Vec<PathBuf>,
    pub last_event_id: u64,
    /// The watch stopped being able to vouch for its root (Linux: queue
    /// overflow, watch limit, permissions, unmount, a removed watch).
    /// The consumer must walk the root fully, with this reason, and must
    /// not treat `changed_dirs` as the whole change. Always `None` from
    /// FSEvents, whose own "rescan" flags are reported as the root.
    pub coverage_lost: Option<(RefreshRefusal, String)>,
    /// Linux: when this watch's epoch opened. A root whose stored
    /// observation predates it is not covered by the watch and needs one
    /// full walk before live batches can be applied incrementally.
    /// `None` from FSEvents, whose replay covers the gap itself.
    pub epoch_opened_at: Option<u64>,
}

/// A running live stream. Dropping it, or calling `stop`, ends the
/// thread and releases the stream.
pub struct Watcher {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Watcher {
    /// A watcher whose thread was started elsewhere in this crate (the
    /// Linux inotify watch, `live_watch::spawn_watch`).
    #[cfg(target_os = "linux")]
    pub(crate) fn from_parts(
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        thread: std::thread::JoinHandle<()>,
    ) -> Self {
        Self {
            stop,
            thread: Some(thread),
        }
    }

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
pub fn watch(root: &Path, tx: std::sync::mpsc::Sender<WatchBatch>) -> Option<Watcher> {
    watch_excluding(root, Vec::new(), tx)
}

/// [`watch`], never reporting a change under `exclude` -- swamp's own
/// store, so writing an observation's results cannot trigger the next
/// live refresh. macOS's stream is unchanged (the exclusion is applied
/// on Linux, where the watcher registers directories itself and so can
/// simply not register those).
pub fn watch_excluding(
    root: &Path,
    exclude: Vec<PathBuf>,
    tx: std::sync::mpsc::Sender<WatchBatch>,
) -> Option<Watcher> {
    #[cfg(target_os = "macos")]
    {
        let _ = exclude;
        macos::watch(root, tx)
    }
    #[cfg(target_os = "linux")]
    {
        crate::live_watch::spawn_watch(root, exclude, tx)
    }
}

/// The non-macOS fallback: always refuses, naming the platform as the
/// cause, never a bug in the replay itself.
///
/// The refusal it gives is the one the platform's continuity source
/// justifies, not a generic "unsupported": on Linux there is no
/// persisted kernel change history to replay from
/// ([`RefreshRefusal::NoPersistedChangeHistory`]), which is a different
/// statement from "nobody has written the backend yet" and leads to a
/// different answer for the user.
pub struct UnsupportedPlatformSource;

impl FsEventsSource for UnsupportedPlatformSource {
    fn replay(&self, _request: &FsEventsRequest) -> FsEventsPlan {
        FsEventsPlan::refuse(platform_refusal(), 0, None)
    }
}

/// Why *this* build cannot replay history. Derived from the platform's
/// continuity source rather than from the target triple, so a future
/// backend changes one table instead of every refusal site.
pub fn platform_refusal() -> RefreshRefusal {
    if crate::platform::ContinuitySource::for_os(crate::platform::Os::current()).replays_history() {
        RefreshRefusal::UnsupportedPlatform
    } else {
        RefreshRefusal::NoPersistedChangeHistory
    }
}

/// Adds `path` and its parent directory to `changes`, provided each is
/// within `root`. FSEvents reports a path whose meaning (the changed item
/// itself, or the directory containing it) depends on flags this design
/// deliberately does not trust for that judgement (see
/// `docs/fsevents-helper.md`'s "every path contributes itself and its
/// parent"); adding both costs one extra listing on re-walk and closes
/// both a creation and a deletion.
#[cfg(target_os = "macos")]
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
        root: PathBuf,
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
            add_with_parent(&mut collector.changes, &collector.root, &path);
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
                root: state.root.clone(),
                changed_dirs: changes.into_iter().collect(),
                last_event_id: last_id,
                coverage_lost: None,
                epoch_opened_at: None,
            });
        }
    }

    pub fn watch(
        root: &Path,
        tx: std::sync::mpsc::Sender<super::WatchBatch>,
    ) -> Option<super::Watcher> {
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
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(true) => Some(super::Watcher {
                stop,
                thread: Some(thread),
            }),
            _ => {
                stop.store(true, Ordering::Relaxed);
                let _ = thread.join();
                None
            }
        }
    }

    pub struct MacOsFsEventsSource;

    impl FsEventsSource for MacOsFsEventsSource {
        fn replay(&self, request: &FsEventsRequest) -> FsEventsPlan {
            replay(&request.root, &request.since)
        }
    }

    fn replay(root: &Path, since: &FsEventsState) -> FsEventsPlan {
        // SAFETY: no arguments; a pure query of the FSEvents subsystem.
        let current = unsafe { fs::FSEventsGetCurrentEventId() };
        if current == 0 {
            return FsEventsPlan::refuse(RefreshRefusal::FseventsdUnavailable, 0, None);
        }

        let dev = match std::fs::metadata(root) {
            Ok(meta) => meta.dev(),
            Err(_) => {
                return FsEventsPlan::refuse(RefreshRefusal::FseventsdUnavailable, current, None);
            }
        };

        // Sanity check named in the issue: ask FSEvents for the last
        // event id it can vouch for on this device as of "now". This is
        // read-only and its only use here is to catch a device whose
        // FSEvents history log cannot possibly reach back to `since`
        // (the id looks plausible but predates everything retained).
        // SAFETY: `dev` came from a real `stat`; `CFAbsoluteTimeGetCurrent`
        // takes no arguments.
        let last_known = unsafe {
            fs::FSEventsGetLastEventIdForDeviceBeforeTime(
                dev,
                core_foundation_sys::date::CFAbsoluteTimeGetCurrent(),
            )
        };

        if let Some(stored_device) = since.device
            && stored_device != dev
        {
            return FsEventsPlan::refuse(RefreshRefusal::RootMismatch, current, Some(dev));
        }

        let Some(since_id) = since.event_id else {
            return FsEventsPlan::refuse(RefreshRefusal::NoStoredEventId, current, Some(dev));
        };
        if since_id > current {
            return FsEventsPlan::refuse(RefreshRefusal::EventIdFromFuture, current, Some(dev));
        }
        // `last_known` being 0 means FSEvents could not answer at all for
        // this device (no history yet observed); that is not by itself a
        // reason to refuse a replay FSEvents is about to attempt, so it
        // only gates the case where FSEvents can positively vouch for a
        // *later* floor than our stored id, meaning `since_id` is stale
        // history that has already rotated out.
        if last_known != 0 && since_id != 0 && since_id < last_known {
            return FsEventsPlan::refuse(RefreshRefusal::HelperInconclusive, current, Some(dev));
        }

        let mut collector = Box::new(Collector {
            root: root.to_path_buf(),
            changes: std::collections::HashSet::new(),
            history_done: false,
            hard_fail: None,
        });
        let info_ptr = collector.as_mut() as *mut Collector as *mut c_void;

        let cf_path = CFString::new(&root.to_string_lossy());
        let paths_array: CFArray<CFString> = CFArray::from_CFTypes(&[cf_path]);
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
            return FsEventsPlan::refuse(RefreshRefusal::FseventsdUnavailable, current, Some(dev));
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
                return FsEventsPlan::refuse(
                    RefreshRefusal::FseventsdUnavailable,
                    current,
                    Some(dev),
                );
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
            return FsEventsPlan::refuse(reason, current, Some(dev));
        }
        if !collector.history_done {
            // The run loop stopped (budget exhausted) without FSEvents
            // ever reporting the replay complete. Whatever was collected
            // may be a prefix of what changed, and a prefix is exactly
            // the silent partial this design refuses to report.
            return FsEventsPlan::refuse(RefreshRefusal::HelperInconclusive, current, Some(dev));
        }

        FsEventsPlan::ok(collector.changes.drain().collect(), current, Some(dev))
    }
}

/// Test-only support kept as an ordinary (non-`#[cfg(test)]`) module so
/// integration tests in `crates/core/tests/*.rs` -- which link against
/// this crate rather than compiling into it -- can use it too. Small and
/// inert in a release binary: one struct, one trait impl, no I/O.
pub mod testing {
    use super::{FsEventsPlan, FsEventsRequest, FsEventsSource};

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
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
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
    fn a_platform_without_a_backend_always_refuses_and_names_which_kind() {
        let source = UnsupportedPlatformSource;
        let plan = source.replay(&FsEventsRequest {
            root: PathBuf::from("/tmp"),
            since: FsEventsState {
                event_id: Some(1),
                device: Some(1),
                last_observed_at: Some(1),
                rules_version: 0,
            },
            swamp_dir: None,
            excluded: Vec::new(),
        });
        assert!(!plan.incremental);
        assert!(
            plan.changed_dirs.is_empty(),
            "a refusal never carries a change list"
        );
        // Which refusal depends on *why* there is no replay. A build on
        // a kernel that keeps no change history says so; only a target
        // with no backend written for it says "unsupported".
        assert_eq!(plan.refusal, Some(platform_refusal()));
        assert_eq!(plan.reason_str(), platform_refusal().as_str());
    }

    /// The distinction that must not collapse: "no backend yet" invites
    /// someone to write one; "the kernel keeps no history" is a fact
    /// about Linux that #81's watcher narrows but does not remove. A
    /// build that reported the first where the second is true would
    /// promise a Linux user an incremental refresh that can never come.
    #[test]
    fn a_kernel_without_persisted_history_says_so_rather_than_unsupported() {
        assert_eq!(
            RefreshRefusal::NoPersistedChangeHistory.as_str(),
            "no_persisted_change_history"
        );
        let why = RefreshRefusal::NoPersistedChangeHistory
            .explanation()
            .expect("this refusal must explain itself in words");
        assert!(why.contains("inotify"), "{why}");
        assert!(
            RefreshRefusal::UnsupportedPlatform.explanation().is_none(),
            "only the platform-history refusal carries that explanation"
        );

        // And the mapping is derived from the continuity source, not
        // from a hand-written per-target list.
        let expected = if crate::platform::ContinuitySource::for_os(crate::platform::Os::current())
            .replays_history()
        {
            RefreshRefusal::UnsupportedPlatform
        } else {
            RefreshRefusal::NoPersistedChangeHistory
        };
        assert_eq!(platform_refusal(), expected);
    }

    #[cfg(target_os = "macos")]
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
            (
                RefreshRefusal::NoPersistedChangeHistory,
                "no_persisted_change_history",
            ),
        ] {
            assert_eq!(reason.as_str(), expected);
        }
    }
}
